use au_plugin_api::{PluginDescriptor, PluginError, PluginFormat, PluginHost, PluginInstance};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum ScanOutcome {
    Found {
        path: PathBuf,
        plugins: Vec<PluginDescriptor>,
    },
    Crashed {
        path: PathBuf,
        status: String,
    },
    TimedOut {
        path: PathBuf,
    },
    Failed {
        path: PathBuf,
        error: String,
    },
}

impl ScanOutcome {
    pub fn path(&self) -> &Path {
        match self {
            Self::Found { path, .. }
            | Self::Crashed { path, .. }
            | Self::TimedOut { path }
            | Self::Failed { path, .. } => path,
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c })
        .collect()
}

pub fn encode(ds: &[PluginDescriptor]) -> String {
    let mut output = String::new();

    for descriptor in ds {
        let format = match descriptor.format {
            PluginFormat::Clap => "CLAP",
            PluginFormat::Vst3 => "VST3",
            PluginFormat::Ladspa => "LADSPA",
            PluginFormat::Lv2 => "LV2",
            _ => continue,
        };
        let path = descriptor
            .path
            .as_deref()
            .unwrap_or_else(|| Path::new(""))
            .display()
            .to_string();

        output.push_str(&format!(
            "PLUGIN\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            format,
            sanitize(&descriptor.id),
            sanitize(&descriptor.name),
            sanitize(&descriptor.vendor),
            sanitize(&path),
            descriptor.n_inputs,
            descriptor.n_outputs
        ));

        for param in &descriptor.params {
            output.push_str(&format!(
                "PARAM\t{}\t{}\t{}\t{}\t{}\n",
                param.id,
                sanitize(&param.name),
                param.min,
                param.max,
                param.default
            ));
        }
    }

    output.push_str("END\n");
    output
}

pub fn decode(s: &str) -> Result<Vec<PluginDescriptor>, String> {
    let mut plugins = Vec::new();
    let mut ended = false;

    for (line_number, line) in s.lines().enumerate() {
        if line.is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        match fields.first().copied() {
            Some("END") if fields.len() == 1 => {
                ended = true;
                break;
            }
            Some("ERROR") => {
                return Err(fields.get(1..).unwrap_or_default().join("\t"));
            }
            Some("PLUGIN") if fields.len() == 8 => {
                let format = match fields[1] {
                    "CLAP" => PluginFormat::Clap,
                    "VST3" => PluginFormat::Vst3,
                    "LADSPA" => PluginFormat::Ladspa,
                    "LV2" => PluginFormat::Lv2,
                    other => {
                        return Err(format!(
                            "invalid plugin format on line {}: {other}",
                            line_number + 1
                        ));
                    }
                };
                let n_inputs = fields[6].parse::<u32>().map_err(|e| {
                    format!("invalid input count on line {}: {e}", line_number + 1)
                })?;
                let n_outputs = fields[7].parse::<u32>().map_err(|e| {
                    format!("invalid output count on line {}: {e}", line_number + 1)
                })?;

                plugins.push(PluginDescriptor {
                    id: fields[2].to_owned(),
                    name: fields[3].to_owned(),
                    vendor: fields[4].to_owned(),
                    format,
                    path: Some(PathBuf::from(fields[5])),
                    params: Vec::new(),
                    n_inputs,
                    n_outputs,
                });
            }
            Some("PARAM") if fields.len() == 6 => {
                let descriptor = plugins.last_mut().ok_or_else(|| {
                    format!("PARAM without a preceding PLUGIN on line {}", line_number + 1)
                })?;
                let id = fields[1].parse::<u32>().map_err(|e| {
                    format!("invalid parameter id on line {}: {e}", line_number + 1)
                })?;
                let min = fields[3].parse::<f32>().map_err(|e| {
                    format!("invalid parameter minimum on line {}: {e}", line_number + 1)
                })?;
                let max = fields[4].parse::<f32>().map_err(|e| {
                    format!("invalid parameter maximum on line {}: {e}", line_number + 1)
                })?;
                let default = fields[5].parse::<f32>().map_err(|e| {
                    format!("invalid parameter default on line {}: {e}", line_number + 1)
                })?;

                descriptor.params.push(au_plugin_api::ParamInfo {
                    id,
                    name: fields[2].to_owned(),
                    min,
                    max,
                    default,
                });
            }
            _ => {
                return Err(format!("invalid scanner output on line {}", line_number + 1));
            }
        }
    }

    if !ended {
        return Err("missing END".to_owned());
    }

    Ok(plugins)
}

pub fn scanner_main(args: &[String]) -> i32 {
    if args.len() != 3 || args[0] != "--format" {
        eprintln!("usage: au-plugin-scan --format <clap|vst3|ladspa|lv2> <path>");
        return 2;
    }

    let format = match args[1].as_str() {
        "clap" => PluginFormat::Clap,
        "vst3" => PluginFormat::Vst3,
        "ladspa" => PluginFormat::Ladspa,
        "lv2" => PluginFormat::Lv2,
        _ => {
            eprintln!("usage: au-plugin-scan --format <clap|vst3|ladspa|lv2> <path>");
            return 2;
        }
    };

    if std::env::var("AU_SCAN_TEST_CRASH").as_deref() == Ok("1")
        && std::env::var("AU_SCAN_TEST_CRASH_FORMAT")
            .map(|value| value == args[1])
            .unwrap_or(true)
    {
        std::process::abort();
    }

    let path = PathBuf::from(&args[2]);
    let result = au_runtime::pool::isolate(|| {
        let descriptors = match format {
            PluginFormat::Clap => crate::clap::scan_file(&path),
            PluginFormat::Vst3 => crate::vst3::scan_file(&path),
            PluginFormat::Ladspa => Ok(crate::ladspa::LadspaHost::new().scan(&[path.clone()])),
            PluginFormat::Lv2 => {
                let mut descriptors = crate::lv2::Lv2Host::new().scan(&[path.clone()]);
                for descriptor in &mut descriptors {
                    if descriptor.path.is_none() {
                        descriptor.path = Some(path.clone());
                    }
                }
                Ok(descriptors)
            }
            _ => unreachable!(),
        };
        descriptors.map_err(|e| au_runtime::JobError::Failed(e.to_string()))
    });

    match result {
        Ok(descriptors) => {
            print!("{}", encode(&descriptors));
            0
        }
        Err(error) => {
            println!("ERROR\t{}\nEND", sanitize(&error.to_string()));
            3
        }
    }
}

pub fn scanner_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AU_PLUGIN_SCANNER") {
        return Some(PathBuf::from(path));
    }

    let executable = std::env::current_exe().ok()?;
    let name = format!("au-plugin-scan{}", std::env::consts::EXE_SUFFIX);
    let parent = executable.parent()?;
    let beside_executable = parent.join(&name);
    if beside_executable.is_file() {
        return Some(beside_executable);
    }

    let beside_parent = parent.parent()?.join(name);
    if beside_parent.is_file() {
        return Some(beside_parent);
    }

    None
}

fn exit_status_description(status: std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }

    format!("exit code {}", status.code().unwrap_or(-1))
}

fn scanner_error_line(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut fields = line.split('\t');
        (fields.next() == Some("ERROR")).then(|| fields.collect::<Vec<_>>().join("\t"))
    })
}

pub fn scan_one(format: PluginFormat, path: &Path, timeout: Duration) -> ScanOutcome {
    let failed = |error: String| ScanOutcome::Failed {
        path: path.to_path_buf(),
        error,
    };

    let Some(scanner) = scanner_path() else {
        return failed("plugin scanner executable not found".to_owned());
    };

    let format_arg = match format {
        PluginFormat::Clap => "clap",
        PluginFormat::Vst3 => "vst3",
        PluginFormat::Ladspa => "ladspa",
        PluginFormat::Lv2 => "lv2",
        _ => return failed(format!("unsupported scan format: {format:?}")),
    };

    let mut child = match Command::new(scanner)
        .arg("--format")
        .arg(format_arg)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return failed(format!("failed to start plugin scanner: {error}")),
    };

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return failed("plugin scanner stdout pipe was unavailable".to_owned());
    };

    let reader = match thread::Builder::new()
        .name("au-plugin-scan-stdout".to_owned())
        .spawn(move || {
            let mut output = String::new();
            stdout.read_to_string(&mut output).map(|_| output)
        }) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return failed(format!("failed to read plugin scanner output: {error}"));
        }
    };

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                au_runtime::crash::log_action(format!(
                    "Plugin scan timed out: {}",
                    path.display()
                ));
                return ScanOutcome::TimedOut {
                    path: path.to_path_buf(),
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return failed(format!("failed waiting for plugin scanner: {error}"));
            }
        }
    };

    let output = match reader.join() {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return failed(format!("failed reading plugin scanner output: {error}"));
        }
        Err(_) => return failed("plugin scanner output reader panicked".to_owned()),
    };

    if !status.success() {
        if status.code() == Some(3) {
            if let Some(error) = scanner_error_line(&output) {
                return failed(error);
            }
        }

        let description = exit_status_description(status);
        au_runtime::crash::log_action(format!(
            "Plugin scan crashed: {} ({description})",
            path.display()
        ));
        return ScanOutcome::Crashed {
            path: path.to_path_buf(),
            status: description,
        };
    }

    match decode(&output) {
        Ok(plugins) => ScanOutcome::Found {
            path: path.to_path_buf(),
            plugins,
        },
        Err(error) => failed(error),
    }
}

fn ladspa_candidates(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn is_ladspa_file(path: &Path) -> bool {
        let is_candidate_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".so") || name.contains(".so."))
            .unwrap_or(false);
        is_candidate_name && path.is_file()
    }

    let mut candidates = Vec::new();
    for path in paths {
        if is_ladspa_file(path) {
            candidates.push(path.clone());
        } else if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let candidate = entry.path();
                    if is_ladspa_file(&candidate) {
                        candidates.push(candidate);
                    }
                }
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn lv2_candidates(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .iter()
        .filter(|path| path.is_dir() && seen.insert((*path).clone()))
        .cloned()
        .collect()
}

fn candidates(format: PluginFormat, paths: &[PathBuf]) -> Vec<PathBuf> {
    match format {
        PluginFormat::Clap => crate::clap::candidates(paths),
        PluginFormat::Vst3 => crate::vst3::candidates(paths),
        PluginFormat::Ladspa => ladspa_candidates(paths),
        PluginFormat::Lv2 => lv2_candidates(paths),
        _ => Vec::new(),
    }
}

pub fn scan_all(
    format: PluginFormat,
    paths: &[PathBuf],
    rt: &au_runtime::Runtime,
    timeout: Duration,
) -> Vec<ScanOutcome> {
    let candidates = candidates(format, paths);

    let jobs: Vec<_> = candidates
        .into_iter()
        .map(|path| {
            let job_path = path.clone();
            let handle = rt.submit(1, move |_ctx| {
                Ok(scan_one(format, &job_path, timeout))
            });
            (path, handle)
        })
        .collect();

    jobs.into_iter()
        .map(|(path, handle)| match handle.wait() {
            Ok(outcome) => outcome,
            Err(error) => ScanOutcome::Failed {
                path,
                error: error.to_string(),
            },
        })
        .collect()
}

pub struct SandboxedHost {
    inner: Box<dyn PluginHost>,
    timeout: Duration,
    last: Mutex<Vec<ScanOutcome>>,
}

impl SandboxedHost {
    pub fn new(inner: impl PluginHost + 'static) -> Self {
        Self {
            inner: Box::new(inner),
            timeout: Duration::from_secs(30),
            last: Mutex::new(Vec::new()),
        }
    }

    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    pub fn last_outcomes(&self) -> Vec<ScanOutcome> {
        self.last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl PluginHost for SandboxedHost {
    fn format(&self) -> PluginFormat {
        self.inner.format()
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        self.inner.default_search_paths()
    }

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        if scanner_path().is_none() {
            au_runtime::crash::log_action(format!(
                "Plugin scanner not found; scanning {:?} in process",
                self.inner.format()
            ));
            self.last
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            return self.inner.scan(paths);
        }

        let runtime = au_runtime::Runtime::new(0, 64);
        let outcomes = scan_all(self.inner.format(), paths, &runtime, self.timeout);
        let mut plugins = Vec::new();

        for outcome in &outcomes {
            if let ScanOutcome::Found {
                plugins: found, ..
            } = outcome
            {
                plugins.extend(found.iter().cloned());
            }
        }

        *self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = outcomes;

        plugins
    }

    fn instantiate(
        &self,
        descriptor: &PluginDescriptor,
    ) -> Result<Box<dyn PluginInstance>, PluginError> {
        self.inner.instantiate(descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip_sanitizes_tabs() {
        let descriptor = PluginDescriptor {
            id: "CLAP:/test.clap;uid".to_owned(),
            name: "A\tB".to_owned(),
            vendor: "Vendor".to_owned(),
            format: PluginFormat::Clap,
            path: Some(PathBuf::from("/test.clap")),
            params: vec![au_plugin_api::ParamInfo {
                id: 7,
                name: "Gain".to_owned(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
            }],
            n_inputs: 2,
            n_outputs: 2,
        };

        let decoded = decode(&encode(&[descriptor])).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].name, "A B");
        assert_eq!(decoded[0].params[0].id, 7);
    }

    #[test]
    fn encode_decode_round_trip_ladspa_and_lv2() {
        let descriptors = [
            PluginDescriptor {
                id: "ladspa:test".to_owned(),
                name: "LADSPA plugin".to_owned(),
                vendor: "Vendor".to_owned(),
                format: PluginFormat::Ladspa,
                path: Some(PathBuf::from("/test.so")),
                params: Vec::new(),
                n_inputs: 1,
                n_outputs: 2,
            },
            PluginDescriptor {
                id: "lv2:test".to_owned(),
                name: "LV2 plugin".to_owned(),
                vendor: "Vendor".to_owned(),
                format: PluginFormat::Lv2,
                path: Some(PathBuf::from("/test.lv2")),
                params: Vec::new(),
                n_inputs: 2,
                n_outputs: 2,
            },
        ];

        let decoded = decode(&encode(&descriptors)).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].format, PluginFormat::Ladspa);
        assert_eq!(decoded[0].id, descriptors[0].id);
        assert_eq!(decoded[1].format, PluginFormat::Lv2);
        assert_eq!(decoded[1].id, descriptors[1].id);
    }

    #[test]
    fn ladspa_candidates_picks_only_so_files() {
        let unique = format!(
            "au-plugin-sandbox-ladspa-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&directory).unwrap();

        let so_file = directory.join("plugin.so");
        let versioned_so_file = directory.join("plugin.so.1");
        let other_file = directory.join("plugin.txt");
        std::fs::write(&so_file, []).unwrap();
        std::fs::write(&versioned_so_file, []).unwrap();
        std::fs::write(&other_file, []).unwrap();

        let candidates = ladspa_candidates(std::slice::from_ref(&directory));
        assert_eq!(candidates, vec![so_file, versioned_so_file]);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn decode_rejects_truncated_output() {
        assert!(decode("PLUGIN\tCLAP\tid\tname\tvendor\t/test.clap\t2\t2\n").is_err());
    }
}
