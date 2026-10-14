//! Audacity-style metadata tags and XML template support.

use std::fs;
use std::io;
use std::path::Path;

/// The title tag.
pub const TITLE: &str = "TITLE";
/// The artist tag.
pub const ARTIST: &str = "ARTIST";
/// The album tag.
pub const ALBUM: &str = "ALBUM";
/// The track-number tag.
pub const TRACK: &str = "TRACKNUMBER";
/// The year tag.
pub const YEAR: &str = "YEAR";
/// The genre tag.
pub const GENRE: &str = "GENRE";
/// The comments tag.
pub const COMMENTS: &str = "COMMENTS";

/// The ID3v1 genre names, in their standard numeric order.
pub const GENRES: &[&str] = &[
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alt. Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta Rap",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
    "Folk",
    "Folk/Rock",
    "National Folk",
    "Swing",
    "Fast-Fusion",
    "Bebob",
    "Latin",
    "Revival",
    "Celtic",
    "Bluegrass",
    "Avantgarde",
    "Gothic Rock",
    "Progressive Rock",
    "Psychedelic Rock",
    "Symphonic Rock",
    "Slow Rock",
    "Big Band",
    "Chorus",
    "Easy Listening",
    "Acoustic",
    "Humour",
    "Speech",
    "Chanson",
    "Opera",
    "Chamber Music",
    "Sonata",
    "Symphony",
    "Booty Bass",
    "Primus",
    "Porn Groove",
    "Satire",
    "Slow Jam",
    "Club",
    "Tango",
    "Samba",
    "Folklore",
    "Ballad",
    "Power Ballad",
    "Rhythmic Soul",
    "Freestyle",
    "Duet",
    "Punk Rock",
    "Drum Solo",
    "A Cappella",
    "Euro-House",
    "Dance Hall",
    "Goa",
    "Drum & Bass",
    "Club-House",
    "Hardcore",
    "Terror",
    "Indie",
    "BritPop",
    "Offensive",
    "Polsk Punk",
    "Beat",
    "Christian Gangsta Rap",
    "Heavy Metal",
    "Black Metal",
    "Crossover",
    "Contemporary Christian",
    "Christian Rock",
    "Merengue",
    "Salsa",
    "Thrash Metal",
    "Anime",
    "JPop",
    "Synthpop",
];

/// Returns the ID3v1 index of a genre name, ignoring ASCII letter case.
pub fn genre_index(name: &str) -> Option<usize> {
    GENRES
        .iter()
        .position(|genre| genre.eq_ignore_ascii_case(name))
}

/// Metadata tags, retaining their insertion order and the spelling of each name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    entries: Vec<(String, String)>,
}

impl Tags {
    /// Creates an empty tag collection.
    /// Track title tag name.
    pub const TITLE: &'static str = TITLE;
    /// Artist tag name.
    pub const ARTIST: &'static str = ARTIST;
    /// Album tag name.
    pub const ALBUM: &'static str = ALBUM;
    /// Track number tag name.
    pub const TRACK: &'static str = TRACK;
    /// Year tag name.
    pub const YEAR: &'static str = YEAR;
    /// Genre tag name.
    pub const GENRE: &'static str = GENRE;
    /// Comments tag name.
    pub const COMMENTS: &'static str = COMMENTS;

    pub fn new() -> Self {
        Self::default()
    }

    /// Gets a tag value by name, ignoring ASCII letter case.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(entry_name, _)| entry_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Sets a tag, replacing any existing tag with the same name regardless of case.
    ///
    /// Empty names and non-ASCII names are ignored. An empty value removes the
    /// tag. Numeric values for `GENRE` are converted to their ID3v1 genre name;
    /// an out-of-range numeric genre removes the tag.
    pub fn set(&mut self, name: &str, value: &str) {
        if name.is_empty() || !name.is_ascii() {
            return;
        }

        let value = if name.eq_ignore_ascii_case(GENRE) {
            match parse_integer(value) {
                Some(index) => GENRES.get(index).copied().unwrap_or("").to_owned(),
                None => value.to_owned(),
            }
        } else {
            value.to_owned()
        };

        if let Some(index) = self
            .entries
            .iter()
            .position(|(entry_name, _)| entry_name.eq_ignore_ascii_case(name))
        {
            if value.is_empty() {
                self.entries.remove(index);
            } else {
                self.entries[index] = (name.to_owned(), value);
            }
        } else if !value.is_empty() {
            self.entries.push((name.to_owned(), value));
        }
    }

    /// Returns the tags as owned name/value pairs in insertion order.
    pub fn entries(&self) -> Vec<(String, String)> {
        self.entries.clone()
    }

    /// Returns whether the standard title, artist, and album tags are all absent.
    ///
    /// This follows Audacity's `Tags::IsEmpty` semantics: other tags alone do
    /// not make the collection non-empty.
    pub fn is_empty(&self) -> bool {
        self.get(TITLE).is_none() && self.get(ARTIST).is_none() && self.get(ALBUM).is_none()
    }

    /// Removes a tag by name, ignoring ASCII letter case.
    pub fn remove(&mut self, name: &str) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(entry_name, _)| entry_name.eq_ignore_ascii_case(name))
        {
            self.entries.remove(index);
        }
    }

    /// Removes all tags.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Produces the `<tags>` XML fragment written by Audacity's tag model.
    pub fn to_xml(&self) -> String {
        let mut xml = String::from("<tags>");
        for (name, value) in &self.entries {
            xml.push_str("<tag name=\"");
            escape_attribute(name, &mut xml);
            xml.push_str("\" value=\"");
            escape_attribute(value, &mut xml);
            xml.push_str("\"/>");
        }
        xml.push_str("</tags>");
        xml
    }

    /// Adds a `<tag>` element from its XML attributes.
    ///
    /// Attributes other than `name` and `value` are ignored. Missing attributes
    /// default to empty strings, and the obsolete `id3v2` tag is ignored.
    pub fn from_xml_attrs(&mut self, pairs: &[(String, String)]) {
        let mut name = "";
        let mut value = "";
        for (attribute, attribute_value) in pairs {
            match attribute.as_str() {
                "name" => name = attribute_value,
                "value" => value = attribute_value,
                _ => {}
            }
        }

        if !name.eq_ignore_ascii_case("id3v2") {
            self.set(name, value);
        }
    }

    /// Saves the tags to a standalone XML template file.
    pub fn save_template<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut contents = String::from("<?xml version=\"1.0\"?>\n");
        contents.push_str(&self.to_xml());
        contents.push('\n');
        fs::write(path, contents)
    }

    /// Loads tags from a standalone XML template file.
    pub fn load_template<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        parse_tags_xml(&contents).map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
    }
}

fn parse_integer(value: &str) -> Option<usize> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Only treat a complete signed decimal integer as an ID3 genre number.
    let digits = trimmed
        .strip_prefix(['+', '-'])
        .unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let parsed: i64 = trimmed.parse().ok()?;
    if parsed < 0 {
        // A numeric but invalid genre is represented by an empty result.
        return Some(usize::MAX);
    }
    usize::try_from(parsed).ok()
}

fn escape_attribute(value: &str, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

fn decode_entities(value: &str) -> Result<String, &'static str> {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(ampersand) = rest.find('&') {
        output.push_str(&rest[..ampersand]);
        let entity_start = ampersand + 1;
        let Some(semicolon_relative) = rest[entity_start..].find(';') else {
            return Err("unterminated XML entity");
        };
        let semicolon = entity_start + semicolon_relative;
        let entity = &rest[entity_start..semicolon];

        match entity {
            "amp" => output.push('&'),
            "lt" => output.push('<'),
            "gt" => output.push('>'),
            "quot" => output.push('"'),
            "apos" => output.push('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                let code = u32::from_str_radix(&entity[2..], 16)
                    .map_err(|_| "invalid hexadecimal XML entity")?;
                output.push(char::from_u32(code).ok_or("invalid XML character entity")?);
            }
            _ if entity.starts_with('#') => {
                let code = entity[1..]
                    .parse::<u32>()
                    .map_err(|_| "invalid decimal XML entity")?;
                output.push(char::from_u32(code).ok_or("invalid XML character entity")?);
            }
            _ => return Err("unknown XML entity"),
        }

        rest = &rest[semicolon + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

fn parse_tags_xml(xml: &str) -> Result<Tags, &'static str> {
    let mut tags = Tags::new();
    let bytes = xml.as_bytes();
    let mut position = 0;

    while position < bytes.len() {
        let Some(relative_start) = xml[position..].find('<') else {
            break;
        };
        let start = position + relative_start;

        if xml[start..].starts_with("<!--") {
            let Some(end) = xml[start + 4..].find("-->") else {
                return Err("unterminated XML comment");
            };
            position = start + 4 + end + 3;
            continue;
        }

        if xml[start..].starts_with("<?") || xml[start..].starts_with("<!") {
            let Some(end) = xml[start..].find('>') else {
                return Err("unterminated XML declaration");
            };
            position = start + end + 1;
            continue;
        }

        let Some(end) = find_tag_end(bytes, start + 1) else {
            return Err("unterminated XML tag");
        };
        let body = xml[start + 1..end].trim();
        position = end + 1;

        if body.is_empty() || body.starts_with('/') {
            continue;
        }

        let (tag_name, attributes) = parse_start_tag(body)?;
        if tag_name == "tag" {
            tags.from_xml_attrs(&attributes);
        }
    }

    Ok(tags)
}

fn find_tag_end(bytes: &[u8], mut position: usize) -> Option<usize> {
    let mut quote = None;
    while position < bytes.len() {
        let byte = bytes[position];
        match (quote, byte) {
            (Some(current), value) if current == value => quote = None,
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return Some(position),
            _ => {}
        }
        position += 1;
    }
    None
}

fn parse_start_tag(body: &str) -> Result<(String, Vec<(String, String)>), &'static str> {
    let body = body.trim_end_matches('/').trim();
    let mut cursor = 0;

    while cursor < body.len() && !body.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor == 0 {
        return Err("missing XML element name");
    }
    let name = body[..cursor].to_owned();
    let mut attributes = Vec::new();

    while cursor < body.len() {
        while cursor < body.len() && body.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == body.len() {
            break;
        }

        let attribute_start = cursor;
        while cursor < body.len()
            && !body.as_bytes()[cursor].is_ascii_whitespace()
            && body.as_bytes()[cursor] != b'='
        {
            cursor += 1;
        }
        if attribute_start == cursor {
            return Err("invalid XML attribute");
        }
        let attribute_name = body[attribute_start..cursor].to_owned();

        while cursor < body.len() && body.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == body.len() || body.as_bytes()[cursor] != b'=' {
            return Err("XML attribute has no value");
        }
        cursor += 1;

        while cursor < body.len() && body.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == body.len() {
            return Err("XML attribute has no quoted value");
        }

        let quote = body.as_bytes()[cursor];
        if quote != b'"' && quote != b'\'' {
            return Err("XML attribute value is not quoted");
        }
        cursor += 1;
        let value_start = cursor;
        while cursor < body.len() && body.as_bytes()[cursor] != quote {
            cursor += 1;
        }
        if cursor == body.len() {
            return Err("unterminated XML attribute value");
        }

        let value = decode_entities(&body[value_start..cursor])?;
        cursor += 1;
        attributes.push((attribute_name, value));
    }

    Ok((name, attributes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_case_insensitive_and_keep_typed_spelling() {
        let mut tags = Tags::new();
        tags.set("Title", "A song");

        assert_eq!(tags.get("TITLE"), Some("A song"));
        assert_eq!(tags.get("title"), Some("A song"));
        assert_eq!(tags.entries(), vec![("Title".to_owned(), "A song".to_owned())]);

        tags.set("TITLE", "Another song");
        assert_eq!(tags.entries(), vec![("TITLE".to_owned(), "Another song".to_owned())]);
    }

    #[test]
    fn removal_works_by_name_or_empty_value() {
        let mut tags = Tags::new();
        tags.set("Artist", "Someone");
        tags.remove("ARTIST");
        assert_eq!(tags.get("artist"), None);

        tags.set("TITLE", "A song");
        tags.set("title", "");
        assert!(tags.entries().is_empty());
    }

    #[test]
    fn numeric_genres_are_converted() {
        let mut tags = Tags::new();
        tags.set("genre", "17");
        assert_eq!(tags.get("GENRE"), Some("Rock"));
        assert_eq!(genre_index("rOcK"), Some(17));
    }

    #[test]
    fn xml_round_trip_escapes_attributes() {
        let mut tags = Tags::new();
        tags.set("Title", "A & B <song>");
        tags.set("Artist", "The \"Band\"");

        let xml = tags.to_xml();
        assert!(xml.contains("A &amp; B &lt;song&gt;"));
        assert!(xml.contains("The &quot;Band&quot;"));

        let parsed = parse_tags_xml(&xml).unwrap();
        assert_eq!(parsed, tags);
    }
}
