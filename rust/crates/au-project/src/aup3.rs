// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3:
// au3/libraries/au3-project-file-io/ProjectFileIO.cpp, ProjectSerializer.cpp,
// SqliteSampleBlock.cpp; au3-wave-track WaveTrack/WaveClip/Sequence; au3-label-track.

//! Audacity 3.x `.aup3` read/write, autosave journal and crash recovery.
//!
//! Layout (same as Audacity 3.x): SQLite with `PRAGMA application_id = 'AUDY'`,
//! `PRAGMA user_version` = packed Audacity version, tables `project` and
//! `autosave` (`id`, `dict`, `doc` — ProjectSerializer binary XML) and
//! `sampleblocks` (one row per sample block with min/max/rms and 256/64k summaries).

use crate::binxml::{self, AttrValue as V, XmlNode};
use crate::blocks::{self, SampleFormat};
use crate::{ProjectError, Result};
use au_core::{
    timeline::{Clip, EnvPoint, Label, LabelTrack, WaveClips},
    AudioBuffer, ProjectState, Track,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// `PACK('A','U','D','Y')`.
pub const APPLICATION_ID: i32 = 0x4155_4459;
/// Base Audacity 3.0.0 project format.
pub const VERSION_3_0: u32 = 0x0300_0000;
/// Audacity 3.4 introduced clip time-stretching (`clipStretchRatio`).
pub const VERSION_3_4: u32 = 0x0304_0000;
/// Newest project format this port reads (the Audacity 3.7 line).
pub const MAX_READ_VERSION: u32 = 0x0307_FFFF;

const HEADER_RAW: [&str; 10] = [
    "<?xml ",
    "version=\"1.0\" ",
    "standalone=\"no\" ",
    "?>\n",
    "<!DOCTYPE ",
    "project ",
    "PUBLIC ",
    "\"-//audacityproject-1.3.0//DTD//EN\" ",
    "\"http://audacity.sourceforge.net/xml/audacityproject-1.3.0.dtd\" ",
    ">\n",
];

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS project(  id                   INTEGER PRIMARY KEY,  dict                 BLOB,  doc                  BLOB);
CREATE TABLE IF NOT EXISTS autosave(  id                   INTEGER PRIMARY KEY,  dict                 BLOB,  doc                  BLOB);
CREATE TABLE IF NOT EXISTS sampleblocks(  blockid              INTEGER PRIMARY KEY AUTOINCREMENT,  sampleformat         INTEGER,  summin               REAL,  summax               REAL,  sumrms               REAL,  summary256           BLOB,  summary64k           BLOB,  samples              BLOB);
";

/// One realtime effect as Audacity stores it (`<effect>` element).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RealtimeEffect {
    /// Plugin id, e.g. `Effect_Audacity_Audacity_Echo_Built-in Effect: Echo`.
    pub id: String,
    /// Whether the effect is enabled.
    pub active: bool,
    /// `(name, value)` parameter strings.
    pub params: Vec<(String, String)>,
}

/// A realtime effect list (`<effects active=...>`), per track or master.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RealtimeEffectList {
    /// List-level bypass flag.
    pub active: bool,
    /// Effects in processing order.
    pub effects: Vec<RealtimeEffect>,
}

fn effects_from_xml(n: &XmlNode) -> RealtimeEffectList {
    RealtimeEffectList {
        active: n.get("active").and_then(V::as_bool).unwrap_or(true),
        effects: n.children_named("effect").map(|e| RealtimeEffect {
            id: e.str("id").unwrap_or("").to_string(),
            active: e.get("active").and_then(V::as_bool).unwrap_or(true),
            params: e.children_named("parameters").flat_map(|ps| ps.children_named("parameter"))
                .map(|p| (p.str("name").unwrap_or("").to_string(), p.str("value").unwrap_or("").to_string())).collect(),
        }).collect(),
    }
}

fn effects_to_xml(l: &RealtimeEffectList) -> XmlNode {
    let mut n = XmlNode::new("effects").attr("active", V::Bool(l.active));
    for e in &l.effects {
        let mut en = XmlNode::new("effect").attr("active", V::Bool(e.active)).attr("id", V::Str(e.id.clone()));
        let mut ps = XmlNode::new("parameters");
        for (k, v) in &e.params {
            ps.children.push(XmlNode::new("parameter").attr("name", V::Str(k.clone())).attr("value", V::Str(v.clone())));
        }
        en.children.push(ps);
        n.children.push(en);
    }
    n
}

/// Data kept in an `.aup3` file that has no home in [`ProjectState`].
#[derive(Clone, Debug, PartialEq)]
pub struct Aup3Extras {
    /// Time selection `(sel0, sel1)` in seconds.
    pub selection: (f64, f64),
    /// Storage format per wave track (index matches `ProjectState::tracks`).
    pub track_formats: Vec<SampleFormat>,
    /// Per wave track `isSelected`.
    pub track_selected: Vec<bool>,
    /// `audacityversion` attribute of the file that was read.
    pub audacity_version: String,
    /// `PRAGMA user_version` of the file that was read.
    pub user_version: u32,
    /// Realtime effect list per wave track (index matches `ProjectState::tracks`; empty list = none).
    pub track_effects: Vec<RealtimeEffectList>,
    /// Master (project) realtime effect list.
    pub master_effects: RealtimeEffectList,
}

impl Default for Aup3Extras {
    fn default() -> Self {
        Self {
            selection: (0.0, 0.0),
            track_formats: Vec::new(),
            track_selected: Vec::new(),
            audacity_version: "3.7.3".into(),
            user_version: VERSION_3_0,
            track_effects: Vec::new(),
            master_effects: RealtimeEffectList::default(),
        }
    }
}

/// A decoded `.aup3` project.
#[derive(Clone, Debug, PartialEq)]
pub struct Aup3Project {
    /// Editable project content.
    pub state: ProjectState,
    /// Format-specific extras.
    pub extras: Aup3Extras,
}

/// Which document to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocSource {
    /// The last explicitly saved document (`project` table).
    Saved,
    /// The autosave journal (`autosave` table), i.e. crash recovery.
    Autosave,
}

/// What [`inspect`] found about a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Aup3Info {
    /// Packed version from `PRAGMA user_version`.
    pub user_version: u32,
    /// A saved `project` document exists.
    pub has_saved: bool,
    /// A non-empty `autosave` document exists: the last session ended
    /// uncleanly (crash or kill) and its unsaved changes can be recovered.
    pub needs_recovery: bool,
    /// Sample blocks referenced by neither document — left by a session that
    /// died after recording/editing (Audacity reports such a file as "not
    /// saved properly" and deletes them on open). Saving drops them.
    pub orphan_blocks: usize,
}

fn bad(m: impl Into<String>) -> ProjectError {
    ProjectError::BadFormat(m.into())
}

fn open_checked(path: &Path) -> Result<(Connection, u32)> {
    let c = Connection::open(path)?;
    let app: i32 = c.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    if app != APPLICATION_ID {
        return Err(bad("not an Audacity project (application_id)"));
    }
    let ver: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let ver = ver as u32;
    if ver > MAX_READ_VERSION {
        return Err(bad(format!(
            "project was saved by a newer Audacity ({}.{}.{}); this build reads up to 3.7",
            ver >> 24,
            (ver >> 16) & 0xff,
            (ver >> 8) & 0xff
        )));
    }
    Ok((c, ver))
}

fn read_doc(c: &Connection, table: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let row: Option<(Option<Vec<u8>>, Option<Vec<u8>>)> = c
        .query_row(&format!("SELECT dict, doc FROM {table} WHERE id = 1"), [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    Ok(match row {
        Some((Some(d), Some(doc))) if !doc.is_empty() => Some((d, doc)),
        _ => None,
    })
}

/// Reports the version and whether an unclean autosave is pending.
pub fn inspect(path: &Path) -> Result<Aup3Info> {
    let (c, user_version) = open_checked(path)?;
    Ok(Aup3Info {
        user_version,
        has_saved: read_doc(&c, "project")?.is_some(),
        needs_recovery: read_doc(&c, "autosave")?.is_some(),
        orphan_blocks: count_orphans(&c)?,
    })
}

/// Loads the saved document (Audacity's behaviour when there is no autosave).
pub fn load(path: &Path) -> Result<Aup3Project> {
    load_from(path, DocSource::Saved)
}

/// Loads either the saved document or the autosave journal.
pub fn load_from(path: &Path, source: DocSource) -> Result<Aup3Project> {
    let (c, ver) = open_checked(path)?;
    let table = match source {
        DocSource::Saved => "project",
        DocSource::Autosave => "autosave",
    };
    let (dict, doc) = read_doc(&c, table)?
        .or(if source == DocSource::Saved {
            // Temporary (never saved) projects only have an autosave doc.
            read_doc(&c, "autosave")?
        } else {
            None
        })
        .ok_or_else(|| bad(format!("no {table} document")))?;
    let root = binxml::decode(&dict, &doc).map_err(|e| bad(e.to_string()))?;
    let mut p = from_xml(&root, &c)?;
    p.extras.user_version = ver;
    Ok(p)
}

/// Discards a pending autosave (user chose not to recover) and drops blocks
/// that only it referenced.
pub fn discard_autosave(path: &Path) -> Result<()> {
    let (c, _) = open_checked(path)?;
    c.execute("DELETE FROM autosave", [])?;
    delete_orphans(&c)?;
    Ok(())
}

// ---------------------------------------------------------------- reading

fn read_block(
    c: &Connection,
    cache: &mut HashMap<i64, Vec<f32>>,
    id: i64,
) -> Result<Vec<f32>> {
    if id <= 0 {
        // SilentBlockFile: negative id encodes the length of silence.
        return Ok(vec![0.0; (-id) as usize]);
    }
    if let Some(v) = cache.get(&id) {
        return Ok(v.clone());
    }
    let (fmt, bytes): (i64, Vec<u8>) = c
        .query_row(
            "SELECT sampleformat, samples FROM sampleblocks WHERE blockid = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| bad(format!("missing sample block {id}")))?;
    let fmt = SampleFormat::from_code(fmt as u32)
        .ok_or_else(|| bad(format!("unknown sample format {fmt:#x}")))?;
    let v = blocks::decode_samples(&bytes, fmt);
    cache.insert(id, v.clone());
    Ok(v)
}

struct ReadClip {
    offset: f64,
    name: String,
    stretch: f64,
    samples: Vec<f32>,
    env: Vec<(f64, f64)>,
}

fn read_clip(
    n: &XmlNode,
    rate: f64,
    c: &Connection,
    cache: &mut HashMap<i64, Vec<f32>>,
) -> Result<ReadClip> {
    let offset = n.f64("offset").unwrap_or(0.0);
    let stretch = n.f64("clipStretchRatio").unwrap_or(1.0);
    let trim_l = n.f64("trimLeft").unwrap_or(0.0);
    let trim_r = n.f64("trimRight").unwrap_or(0.0);
    let seq = n
        .children_named("sequence")
        .next()
        .ok_or_else(|| bad("waveclip without sequence"))?;
    let num = seq.i64("numsamples").unwrap_or(0).max(0) as usize;
    let mut samples = Vec::with_capacity(num);
    for b in seq.children_named("waveblock") {
        let start = b.i64("start").unwrap_or(samples.len() as i64).max(0) as usize;
        let id = b.i64("blockid").ok_or_else(|| bad("waveblock without blockid"))?;
        let data = read_block(c, cache, id)?;
        samples.resize(start, 0.0);
        samples.extend_from_slice(&data);
    }
    samples.resize(num, 0.0);
    // Trims are in (stretched) play time; convert to source samples.
    let to_src = |t: f64| ((t * rate / stretch.max(1e-9)).round().max(0.0)) as usize;
    let (l, r) = (to_src(trim_l).min(samples.len()), to_src(trim_r));
    let end = samples.len().saturating_sub(r).max(l);
    let samples = samples[l..end].to_vec();
    let env = n
        .children_named("envelope")
        .flat_map(|e| e.children_named("controlpoint"))
        .map(|p| (p.f64("t").unwrap_or(0.0), p.f64("val").unwrap_or(1.0)))
        .collect();
    Ok(ReadClip {
        // Audacity's play start = sequence offset + left trim.
        offset: offset + trim_l,
        name: n.str("name").unwrap_or("").to_string(),
        stretch,
        samples,
        env,
    })
}

fn from_xml(root: &XmlNode, c: &Connection) -> Result<Aup3Project> {
    if root.name != "project" {
        return Err(bad(format!("root element is <{}>", root.name)));
    }
    let proj_rate = root.f64("rate").unwrap_or(44100.0);
    let mut extras = Aup3Extras {
        selection: (root.f64("sel0").unwrap_or(0.0), root.f64("sel1").unwrap_or(0.0)),
        audacity_version: root.str("audacityversion").unwrap_or("").to_string(),
        ..Default::default()
    };
    let mut cache = HashMap::new();
    let mut tracks = Vec::new();
    let mut labels = Vec::new();
    let kids: Vec<&XmlNode> = root.children.iter().collect();
    let mut i = 0;
    while i < kids.len() {
        let n = kids[i];
        i += 1;
        match n.name.as_str() {
            "wavetrack" => {
                // Stereo = this element linked to the next one (linked != 0).
                let mut chans = vec![n];
                if n.i64("linked").unwrap_or(0) != 0 && i < kids.len() && kids[i].name == "wavetrack" {
                    chans.push(kids[i]);
                    i += 1;
                }
                let rate = n.f64("rate").unwrap_or(proj_rate);
                let r = rate.round() as u32;
                let fmt = n
                    .i64("sampleformat")
                    .and_then(|f| SampleFormat::from_code(f as u32))
                    .unwrap_or(SampleFormat::Float);
                let per_chan: Vec<Vec<ReadClip>> = chans
                    .iter()
                    .map(|ch| {
                        ch.children_named("waveclip")
                            .map(|cl| read_clip(cl, rate, c, &mut cache))
                            .collect::<Result<Vec<_>>>()
                    })
                    .collect::<Result<_>>()?;
                let mut wc = WaveClips::new(r, chans.len());
                let mut env = Vec::new();
                for (ci, first) in per_chan[0].iter().enumerate() {
                    let mut channels = vec![first.samples.clone()];
                    for other in per_chan.iter().skip(1) {
                        let mut s = other.get(ci).map(|x| x.samples.clone()).unwrap_or_default();
                        s.resize(first.samples.len(), 0.0);
                        channels.push(s);
                    }
                    let mut clip = Clip::new(
                        first.name.clone(),
                        (first.offset * rate).round().max(0.0) as usize,
                        AudioBuffer::from_channels(r, channels),
                    );
                    clip.stretch_ratio = first.stretch;
                    // Control point times are relative to the clip's offset.
                    for &(t, v) in &first.env {
                        env.push(EnvPoint { time: first.offset + t, value: v });
                    }
                    wc.clips.push(clip);
                }
                wc.clips.sort_by_key(|c| c.start);
                env.sort_by(|a, b| a.time.total_cmp(&b.time));
                wc.envelope.points = env;
                tracks.push(Track {
                    name: n.str("name").unwrap_or("Audio").to_string(),
                    clips: wc,
                    gain: n.f64("gain").unwrap_or(1.0) as f32,
                    pan: n.f64("pan").unwrap_or(0.0) as f32,
                    mute: n.get("mute").and_then(V::as_bool).unwrap_or(false),
                    solo: n.get("solo").and_then(V::as_bool).unwrap_or(false),
                });
                extras.track_formats.push(fmt);
                extras.track_effects.push(n.children_named("effects").next().map(effects_from_xml).unwrap_or_default());
                extras
                    .track_selected
                    .push(n.get("isSelected").and_then(V::as_bool).unwrap_or(false));
            }
            "labeltrack" => {
                let mut lt = LabelTrack::new(n.str("name").unwrap_or("Labels"));
                for l in n.children_named("label") {
                    let t = l.f64("t").unwrap_or(0.0);
                    lt.labels.push(Label {
                        start: t,
                        end: l.f64("t1").unwrap_or(t),
                        text: l.str("title").unwrap_or("").to_string(),
                    });
                }
                lt.labels.sort_by(|a, b| a.start.total_cmp(&b.start));
                labels.push(lt);
            }
            "effects" => extras.master_effects = effects_from_xml(n),
            _ => {} // tags, timetrack, notetrack: not modelled yet
        }
    }
    Ok(Aup3Project {
        state: ProjectState {
            tracks,
            sample_rate: proj_rate.round() as u32,
            labels,
            sync_lock: false,
        },
        extras,
    })
}

// ---------------------------------------------------------------- writing

fn insert_block(c: &Connection, samples: &[f32], fmt: SampleFormat) -> Result<i64> {
    let s = blocks::summarize(samples);
    c.execute(
        "INSERT INTO sampleblocks (sampleformat, summin, summax, sumrms, summary256, summary64k, samples)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            fmt.code() as i64,
            s.min as f64,
            s.max as f64,
            s.rms,
            s.summary256,
            s.summary64k,
            blocks::encode_samples(samples, fmt)
        ],
    )?;
    Ok(c.last_insert_rowid())
}

fn d(v: f64, digits: i32) -> V {
    V::Double(v, digits)
}

fn build_xml(c: &Connection, p: &Aup3Project) -> Result<XmlNode> {
    let st = &p.state;
    let rate = if st.sample_rate == 0 { 44100 } else { st.sample_rate } as f64;
    let mut root = XmlNode::new("project")
        .attr("xmlns", V::Str("http://audacity.sourceforge.net/xml/".into()))
        .attr("version", V::Str("1.3.0".into()))
        .attr("audacityversion", V::Str("3.7.3".into()))
        .attr("rate", d(rate, -1))
        .attr("snapto", V::Str("off".into()))
        .attr("sel0", d(p.extras.selection.0, 10))
        .attr("sel1", d(p.extras.selection.1, 10))
        .attr("vpos", V::Int(0))
        .attr("h", d(0.0, 10))
        .attr("zoom", d(86.1328125, 10));
    root.children.push(XmlNode::new("tags"));
    if !p.extras.master_effects.effects.is_empty() {
        root.children.push(effects_to_xml(&p.extras.master_effects));
    }
    for (ti, t) in st.tracks.iter().enumerate() {
        let fmt = p.extras.track_formats.get(ti).copied().unwrap_or(SampleFormat::Float);
        let sel = p.extras.track_selected.get(ti).copied().unwrap_or(false);
        let n_ch = t.clips.clips.iter().map(|c| c.buffer.n_channels()).max().unwrap_or(t.clips.n_channels).clamp(1, 2);
        let trate = t.clips.sample_rate as f64;
        for ch in 0..n_ch {
            let mut wt = XmlNode::new("wavetrack")
                .attr("name", V::Str(t.name.clone()))
                .attr("isSelected", V::Bool(sel))
                .attr("colorindex", V::Int(0))
                .attr("height", V::Int(150))
                .attr("minimized", V::Bool(false))
                .attr("channel", V::Int(ch as i32))
                .attr("linked", V::Int(if n_ch == 2 && ch == 0 { 3 } else { 0 }))
                .attr("mute", V::Bool(t.mute))
                .attr("solo", V::Bool(t.solo))
                .attr("rate", d(trate, -1))
                .attr("gain", d(t.gain as f64, -1))
                .attr("pan", d(t.pan as f64, -1))
                .attr("sampleformat", V::Long(fmt.code() as i32));
            if ch == 0 {
                if let Some(l) = p.extras.track_effects.get(ti).filter(|l| !l.effects.is_empty()) {
                    wt.children.push(effects_to_xml(l));
                }
            }
            for clip in &t.clips.clips {
                let offset = clip.start as f64 / trate;
                let end = offset + clip.len() as f64 / trate;
                let empty: Vec<f32> = Vec::new();
                let data = clip.buffer.channels.get(ch).or(clip.buffer.channels.first()).unwrap_or(&empty);
                let mut seq = XmlNode::new("sequence")
                    .attr("maxsamples", V::SizeT(fmt.max_samples() as u32))
                    .attr("sampleformat", V::SizeT(fmt.code()))
                    .attr("effectivesampleformat", V::SizeT(fmt.code()))
                    .attr("numsamples", V::LongLong(data.len() as i64));
                for (bi, chunk) in data.chunks(fmt.max_samples()).enumerate() {
                    let id = insert_block(c, chunk, fmt)?;
                    seq.children.push(
                        XmlNode::new("waveblock")
                            .attr("start", V::LongLong((bi * fmt.max_samples()) as i64))
                            .attr("blockid", V::LongLong(id)),
                    );
                }
                // Envelope points that fall inside this clip, relative to its offset.
                let pts: Vec<&EnvPoint> = t
                    .clips
                    .envelope
                    .points
                    .iter()
                    .filter(|pt| pt.time >= offset && pt.time <= end)
                    .collect();
                let mut env = XmlNode::new("envelope").attr("numpoints", V::SizeT(pts.len() as u32));
                for pt in pts {
                    env.children.push(
                        XmlNode::new("controlpoint")
                            .attr("t", d(pt.time - offset, 12))
                            .attr("val", d(pt.value, 12)),
                    );
                }
                let wc = XmlNode::new("waveclip")
                    .attr("offset", d(offset, 8))
                    .attr("trimLeft", d(0.0, 8))
                    .attr("trimRight", d(0.0, 8))
                    .attr("centShift", V::Int(0))
                    .attr("pitchAndSpeedPreset", V::Long(0))
                    .attr("clipStretchRatio", d(clip.stretch_ratio, 8))
                    .attr("name", V::Str(clip.name.clone()))
                    .attr("colorindex", V::Int(0))
                    .child(seq)
                    .child(env);
                wt.children.push(wc);
            }
            root.children.push(wt);
        }
    }
    for lt in &st.labels {
        let mut n = XmlNode::new("labeltrack")
            .attr("name", V::Str(lt.name.clone()))
            .attr("isSelected", V::Bool(false))
            .attr("height", V::Int(73))
            .attr("minimized", V::Bool(false))
            .attr("numlabels", V::Int(lt.labels.len() as i32));
        for l in &lt.labels {
            n.children.push(
                XmlNode::new("label")
                    .attr("t", d(l.start, 10))
                    .attr("t1", d(l.end, 10))
                    .attr("title", V::Str(l.text.clone())),
            );
        }
        root.children.push(n);
    }
    Ok(root)
}

fn version_for(p: &Aup3Project) -> u32 {
    let stretched = p
        .state
        .tracks
        .iter()
        .flat_map(|t| &t.clips.clips)
        .any(|c| (c.stretch_ratio - 1.0).abs() > 1e-12);
    let needed = if stretched { VERSION_3_4 } else { VERSION_3_0 };
    // Keep the version of the file we read (avoids Audacity's "older version"
    // upgrade notice) but never claim newer than we can read, nor older than needed.
    needed.max(p.extras.user_version.min(MAX_READ_VERSION & 0xFFFF_0000))
}

fn write_doc(c: &Connection, table: &str, root: &XmlNode) -> Result<()> {
    let enc = binxml::encode(root, 4, &HEADER_RAW);
    c.execute(
        &format!(
            "INSERT INTO {table}(id, dict, doc) VALUES(1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET dict = ?1, doc = ?2"
        ),
        params![enc.dict, enc.doc],
    )?;
    Ok(())
}

fn referenced(c: &Connection, table: &str) -> Result<HashSet<i64>> {
    let mut ids = HashSet::new();
    if let Some((dict, doc)) = read_doc(c, table)? {
        let root = binxml::decode(&dict, &doc).map_err(|e| bad(e.to_string()))?;
        fn walk(n: &XmlNode, ids: &mut HashSet<i64>) {
            if n.name == "waveblock" {
                if let Some(id) = n.i64("blockid") {
                    ids.insert(id);
                }
            }
            n.children.iter().for_each(|k| walk(k, ids));
        }
        walk(&root, &mut ids);
    }
    Ok(ids)
}

fn orphan_ids(c: &Connection) -> Result<Vec<i64>> {
    let mut keep = referenced(c, "project")?;
    keep.extend(referenced(c, "autosave")?);
    let all: Vec<i64> = c
        .prepare("SELECT blockid FROM sampleblocks")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(all.into_iter().filter(|id| !keep.contains(id)).collect())
}

fn count_orphans(c: &Connection) -> Result<usize> {
    Ok(orphan_ids(c)?.len())
}

fn delete_orphans(c: &Connection) -> Result<usize> {
    let mut n = 0;
    for id in orphan_ids(c)? {
        n += c.execute("DELETE FROM sampleblocks WHERE blockid = ?1", params![id])?;
    }
    Ok(n)
}

fn init_schema(c: &Connection, version: u32) -> Result<()> {
    c.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {version};{SCHEMA}"
    ))?;
    Ok(())
}

/// Saves (Save / Save As) a complete, self-contained `.aup3`.
///
/// The file is built next to the target and atomically renamed over it, so a
/// failure never damages an existing project. Any autosave is cleared.
pub fn save(path: &Path, p: &Aup3Project) -> Result<()> {
    let tmp = path.with_extension("aup3.saving");
    let _ = std::fs::remove_file(&tmp);
    {
        let mut c = Connection::open(&tmp)?;
        init_schema(&c, version_for(p))?;
        let tx = c.transaction()?;
        let root = build_xml(&tx, p)?;
        write_doc(&tx, "project", &root)?;
        tx.commit()?;
    }
    for ext in ["aup3-wal", "aup3-shm"] {
        let _ = std::fs::remove_file(path.with_extension(ext));
    }
    std::fs::rename(&tmp, path).map_err(|e| bad(format!("rename: {e}")))?;
    Ok(())
}

/// Saves a plain [`ProjectState`] with default extras (float samples).
pub fn save_state(path: &Path, state: &ProjectState) -> Result<()> {
    save(path, &Aup3Project { state: state.clone(), extras: Aup3Extras::default() })
}

/// Journals the current (unsaved) state into the `autosave` table, like
/// Audacity's `ProjectFileIO::AutoSave` after every undoable edit. The saved
/// `project` document is untouched; blocks only used by the previous autosave
/// are removed. Creates the file (a "temporary project") if it does not exist.
pub fn autosave(path: &Path, p: &Aup3Project) -> Result<()> {
    let mut c = Connection::open(path)?;
    let ver: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    init_schema(&c, (ver as u32).max(version_for(p)))?;
    let tx = c.transaction()?;
    let root = build_xml(&tx, p)?;
    write_doc(&tx, "autosave", &root)?;
    delete_orphans(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Recovers from autosave: loads the journal and writes it as the saved
/// document, leaving a clean file.
pub fn recover(path: &Path) -> Result<Aup3Project> {
    let p = load_from(path, DocSource::Autosave)?;
    save(path, &p)?;
    Ok(p)
}

/// Decodes the saved (or autosave) document to indented XML text, for golden
/// comparisons and debugging.
pub fn dump_xml(path: &Path, source: DocSource) -> Result<String> {
    let (c, _) = open_checked(path)?;
    let t = if source == DocSource::Saved { "project" } else { "autosave" };
    let (dict, doc) = read_doc(&c, t)?.ok_or_else(|| bad("no document"))?;
    Ok(binxml::decode(&dict, &doc).map_err(|e| bad(e.to_string()))?.to_xml_string())
}

/// `PRAGMA integrity_check` result (`"ok"` when healthy).
pub fn integrity_check(path: &Path) -> Result<String> {
    let c = Connection::open(path)?;
    Ok(c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?)
}
