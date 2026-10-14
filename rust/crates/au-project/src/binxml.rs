// SPDX-License-Identifier: GPL-3.0-or-later

//! Decoder and encoder for Audacity ProjectSerializer binary XML.

use std::collections::HashMap;

/// A value stored in an XML attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    /// A string value.
    Str(String),
    /// A 32-bit integer value.
    Int(i32),
    /// A boolean value.
    Bool(bool),
    /// A 32-bit long value.
    Long(i32),
    /// A 64-bit integer value.
    LongLong(i64),
    /// An unsigned size value.
    SizeT(u32),
    /// A 32-bit floating-point value and its precision metadata.
    Float(f32, i32),
    /// A 64-bit floating-point value and its precision metadata.
    Double(f64, i32),
    /// A byte blob.
    Blob(Vec<u8>),
}

impl AttrValue {
    /// Returns this value as an `f64`, parsing strings when possible.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Str(value) => value.parse().ok(),
            Self::Int(value) | Self::Long(value) => Some(f64::from(*value)),
            Self::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
            Self::LongLong(value) => Some(*value as f64),
            Self::SizeT(value) => Some(f64::from(*value)),
            Self::Float(value, _) => Some(f64::from(*value)),
            Self::Double(value, _) => Some(*value),
            Self::Blob(_) => None,
        }
    }

    /// Returns this value as an `i64`, parsing strings when possible.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Str(value) => value.parse().ok(),
            Self::Int(value) | Self::Long(value) => Some(i64::from(*value)),
            Self::Bool(value) => Some(if *value { 1 } else { 0 }),
            Self::LongLong(value) => Some(*value),
            Self::SizeT(value) => Some(i64::from(*value)),
            Self::Float(value, _) => Some(*value as i64),
            Self::Double(value, _) => Some(*value as i64),
            Self::Blob(_) => None,
        }
    }

    /// Returns this value as a string, if it is a string value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(value) => Some(value),
            _ => None,
        }
    }

    /// Returns this value as a boolean when its representation is recognized.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Int(value) | Self::Long(value) => Some(*value != 0),
            Self::LongLong(value) => Some(*value != 0),
            Self::SizeT(value) => Some(*value != 0),
            Self::Str(value) => match value.as_str() {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            },
            Self::Float(_, _) | Self::Double(_, _) | Self::Blob(_) => None,
        }
    }
}

/// A decoded XML element.
#[derive(Clone, Debug, PartialEq)]
pub struct XmlNode {
    /// The element name.
    pub name: String,
    /// Attributes in their original order.
    pub attrs: Vec<(String, AttrValue)>,
    /// Child elements in their original order.
    pub children: Vec<XmlNode>,
    /// Text content preceding the child elements.
    pub text: String,
}

impl XmlNode {
    /// Creates an empty element with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attrs: Vec::new(),
            children: Vec::new(),
            text: String::new(),
        }
    }

    /// Adds an attribute and returns this node for builder-style use.
    pub fn attr(mut self, key: impl Into<String>, value: AttrValue) -> Self {
        self.push_attr(key, value);
        self
    }

    /// Adds an attribute, preserving any existing attributes with the same name.
    pub fn push_attr(&mut self, key: impl Into<String>, value: AttrValue) {
        self.attrs.push((key.into(), value));
    }

    /// Gets the first attribute with the given name.
    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.attrs
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value))
    }

    /// Gets an attribute as an `f64`.
    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    /// Gets an attribute as an `i64`.
    pub fn i64(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    /// Gets an attribute as a string.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// Adds a child and returns this node for builder-style use.
    pub fn child(mut self, child: XmlNode) -> Self {
        self.children.push(child);
        self
    }

    /// Iterates over the direct children with the given name.
    pub fn children_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a XmlNode> {
        self.children
            .iter()
            .filter(move |child| child.name == name)
    }

    /// Formats this node as indented textual XML.
    ///
    /// Floating-point attributes use Rust's standard `{}` formatting.
    pub fn to_xml_string(&self) -> String {
        let mut output = String::new();
        self.write_xml(&mut output, 0);
        output
    }

    fn write_xml(&self, output: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        output.push_str(&indent);
        output.push('<');
        output.push_str(&self.name);

        for (key, value) in &self.attrs {
            output.push(' ');
            output.push_str(key);
            output.push_str("=\"");
            escape_xml_attr(&attr_to_string(value), output);
            output.push('"');
        }

        if self.children.is_empty() && self.text.is_empty() {
            output.push_str("/>\n");
            return;
        }

        output.push('>');
        if self.children.is_empty() {
            escape_xml_text(&self.text, output);
            output.push_str("</");
            output.push_str(&self.name);
            output.push_str(">\n");
            return;
        }

        if !self.text.is_empty() {
            escape_xml_text(&self.text, output);
        }
        output.push('\n');
        for child in &self.children {
            child.write_xml(output, depth + 1);
        }
        output.push_str(&indent);
        output.push_str("</");
        output.push_str(&self.name);
        output.push_str(">\n");
    }
}

fn attr_to_string(value: &AttrValue) -> String {
    match value {
        AttrValue::Str(value) => value.clone(),
        AttrValue::Int(value) | AttrValue::Long(value) => value.to_string(),
        AttrValue::Bool(value) => value.to_string(),
        AttrValue::LongLong(value) => value.to_string(),
        AttrValue::SizeT(value) => value.to_string(),
        AttrValue::Float(value, _) => format!("{value}"),
        AttrValue::Double(value, _) => format!("{value}"),
        AttrValue::Blob(bytes) => {
            let mut output = String::with_capacity(bytes.len().saturating_mul(2));
            for byte in bytes {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
            }
            output
        }
    }
}

fn escape_xml_attr(value: &str, output: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(ch),
        }
    }
}

fn escape_xml_text(value: &str, output: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}

/// Errors that can occur while decoding binary XML.
#[derive(Debug, thiserror::Error)]
pub enum BinXmlError {
    /// The input ended before a field could be read.
    #[error("truncated binary xml at {0}")]
    Truncated(usize),
    /// A field referred to an undefined name.
    #[error("unknown name id {0}")]
    UnknownId(u16),
    /// A character-size marker was not supported.
    #[error("bad char size {0}")]
    BadCharSize(u8),
    /// A field type byte was not recognized.
    #[error("unknown field type {0} at {1}")]
    UnknownType(u8, usize),
    /// Tags or name-map scopes were not balanced.
    #[error("unbalanced tags")]
    Unbalanced,
    /// A string could not be decoded using the selected character size.
    #[error("invalid string")]
    BadString,
}

/// Encoded dictionary and document blobs.
pub struct Encoded {
    /// The name dictionary, beginning with the character-size marker.
    pub dict: Vec<u8>,
    /// The document records.
    pub doc: Vec<u8>,
}

/// Decodes a dictionary and document as one binary XML stream.
pub fn decode(dict: &[u8], doc: &[u8]) -> Result<XmlNode, BinXmlError> {
    let mut reader = Reader::new(dict, doc);

    let marker_offset = reader.position();
    let marker = reader.read_u8()?;
    if marker != 0 {
        return Err(BinXmlError::UnknownType(marker, marker_offset));
    }
    let mut char_size = reader.read_u8()?;
    validate_char_size(char_size)?;

    let mut names = HashMap::<u16, String>::new();
    let mut scopes = Vec::<HashMap<u16, String>>::new();
    let mut node_stack = Vec::<XmlNode>::new();
    let mut tag_id_stack = Vec::<u16>::new();
    let mut root = None;

    while reader.remaining() > 0 {
        let field_offset = reader.position();
        let field_type = reader.read_u8()?;

        match field_type {
            0 => {
                char_size = reader.read_u8()?;
                validate_char_size(char_size)?;
            }
            1 => {
                let id = reader.read_u16()?;
                let name = lookup_name(&names, id)?;
                if node_stack.is_empty() && root.is_some() {
                    return Err(BinXmlError::Unbalanced);
                }
                node_stack.push(XmlNode::new(name));
                tag_id_stack.push(id);
            }
            2 => {
                let id = reader.read_u16()?;
                let _ = lookup_name(&names, id)?;
                let node = node_stack.pop().ok_or(BinXmlError::Unbalanced)?;
                let open_id = tag_id_stack.pop().ok_or(BinXmlError::Unbalanced)?;
                if open_id != id {
                    return Err(BinXmlError::Unbalanced);
                }
                if let Some(parent) = node_stack.last_mut() {
                    parent.children.push(node);
                } else if root.replace(node).is_some() {
                    return Err(BinXmlError::Unbalanced);
                }
            }
            3 => {
                let id = reader.read_u16()?;
                let len = reader.read_i32_len()?;
                let bytes = reader.read_vec(len)?;
                let value = decode_chars(&bytes, char_size)?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Str(value),
                );
            }
            4 => {
                let id = reader.read_u16()?;
                let value = reader.read_i32()?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Int(value),
                );
            }
            5 => {
                let id = reader.read_u16()?;
                let value = reader.read_u8()? != 0;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Bool(value),
                );
            }
            6 => {
                let id = reader.read_u16()?;
                let value = reader.read_i32()?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Long(value),
                );
            }
            7 => {
                let id = reader.read_u16()?;
                let value = reader.read_i64()?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::LongLong(value),
                );
            }
            8 => {
                let id = reader.read_u16()?;
                let value = reader.read_u32()?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::SizeT(value),
                );
            }
            9 => {
                let id = reader.read_u16()?;
                let value = reader.read_f32()?;
                let digits = reader.read_i32()?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Float(value, digits),
                );
            }
            10 => {
                let id = reader.read_u16()?;
                let value = reader.read_f64()?;
                let digits = reader.read_i32()?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Double(value, digits),
                );
            }
            11 => {
                let len = reader.read_i32_len()?;
                let bytes = reader.read_vec(len)?;
                let text = decode_chars(&bytes, char_size)?;
                current_node(&mut node_stack)?.text.push_str(&text);
            }
            12 => {
                let len = reader.read_i32_len()?;
                reader.skip(len)?;
            }
            13 => {
                scopes.push(std::mem::take(&mut names));
            }
            14 => {
                names = scopes.pop().ok_or(BinXmlError::Unbalanced)?;
            }
            15 => {
                let id = reader.read_u16()?;
                let len = usize::from(reader.read_u16()?);
                let bytes = reader.read_vec(len)?;
                let name = decode_chars(&bytes, char_size)?;
                names.insert(id, name);
            }
            16 => {
                let id = reader.read_u16()?;
                let len = reader.read_i32_len()?;
                let bytes = reader.read_vec(len)?;
                current_node(&mut node_stack)?.push_attr(
                    lookup_name(&names, id)?.to_owned(),
                    AttrValue::Blob(bytes),
                );
            }
            other => return Err(BinXmlError::UnknownType(other, field_offset)),
        }
    }

    if !node_stack.is_empty() || !tag_id_stack.is_empty() || !scopes.is_empty() {
        return Err(BinXmlError::Unbalanced);
    }
    root.ok_or(BinXmlError::Unbalanced)
}

fn current_node(stack: &mut [XmlNode]) -> Result<&mut XmlNode, BinXmlError> {
    stack.last_mut().ok_or(BinXmlError::Unbalanced)
}

fn lookup_name(names: &HashMap<u16, String>, id: u16) -> Result<&str, BinXmlError> {
    names
        .get(&id)
        .map(String::as_str)
        .ok_or(BinXmlError::UnknownId(id))
}

fn validate_char_size(char_size: u8) -> Result<(), BinXmlError> {
    match char_size {
        1 | 2 | 4 => Ok(()),
        other => Err(BinXmlError::BadCharSize(other)),
    }
}

fn decode_chars(bytes: &[u8], char_size: u8) -> Result<String, BinXmlError> {
    match char_size {
        1 => String::from_utf8(bytes.to_vec()).map_err(|_| BinXmlError::BadString),
        2 => {
            if bytes.len() % 2 != 0 {
                return Err(BinXmlError::BadString);
            }
            let words = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            String::from_utf16(&words).map_err(|_| BinXmlError::BadString)
        }
        4 => {
            if bytes.len() % 4 != 0 {
                return Err(BinXmlError::BadString);
            }
            let mut result = String::new();
            for chunk in bytes.chunks_exact(4) {
                let value = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                result.push(char::from_u32(value).ok_or(BinXmlError::BadString)?);
            }
            Ok(result)
        }
        other => Err(BinXmlError::BadCharSize(other)),
    }
}

struct Reader<'a> {
    dict: &'a [u8],
    doc: &'a [u8],
    position: usize,
    total: usize,
}

impl<'a> Reader<'a> {
    fn new(dict: &'a [u8], doc: &'a [u8]) -> Self {
        let total = dict.len().checked_add(doc.len()).unwrap_or(usize::MAX);
        Self {
            dict,
            doc,
            position: 0,
            total,
        }
    }

    fn position(&self) -> usize {
        self.position
    }

    fn remaining(&self) -> usize {
        self.total.saturating_sub(self.position)
    }

    fn read_u8(&mut self) -> Result<u8, BinXmlError> {
        if self.position >= self.total {
            return Err(BinXmlError::Truncated(self.total));
        }
        let value = if self.position < self.dict.len() {
            self.dict[self.position]
        } else {
            self.doc[self.position - self.dict.len()]
        };
        self.position += 1;
        Ok(value)
    }

    fn ensure(&self, len: usize) -> Result<(), BinXmlError> {
        if len > self.remaining() {
            Err(BinXmlError::Truncated(self.total))
        } else {
            Ok(())
        }
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], BinXmlError> {
        self.ensure(N)?;
        let mut bytes = [0; N];
        for byte in &mut bytes {
            *byte = self.read_u8()?;
        }
        Ok(bytes)
    }

    fn read_vec(&mut self, len: usize) -> Result<Vec<u8>, BinXmlError> {
        self.ensure(len)?;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(self.read_u8()?);
        }
        Ok(bytes)
    }

    fn skip(&mut self, len: usize) -> Result<(), BinXmlError> {
        self.ensure(len)?;
        self.position += len;
        Ok(())
    }

    fn read_u16(&mut self) -> Result<u16, BinXmlError> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    fn read_u32(&mut self) -> Result<u32, BinXmlError> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    fn read_i32(&mut self) -> Result<i32, BinXmlError> {
        Ok(i32::from_le_bytes(self.read_array()?))
    }

    fn read_i64(&mut self) -> Result<i64, BinXmlError> {
        Ok(i64::from_le_bytes(self.read_array()?))
    }

    fn read_f32(&mut self) -> Result<f32, BinXmlError> {
        Ok(f32::from_le_bytes(self.read_array()?))
    }

    fn read_f64(&mut self) -> Result<f64, BinXmlError> {
        Ok(f64::from_le_bytes(self.read_array()?))
    }

    fn read_i32_len(&mut self) -> Result<usize, BinXmlError> {
        let value = self.read_i32()?;
        usize::try_from(value).map_err(|_| BinXmlError::Truncated(self.position))
    }
}

/// Encodes a node tree into dictionary and document blobs.
///
/// `char_size` must be 1, 2, or 4.
pub fn encode(root: &XmlNode, char_size: u8, header_raw: &[&str]) -> Encoded {
    validate_char_size(char_size).expect("char_size must be 1, 2, or 4");

    let mut name_ids = HashMap::<String, u16>::new();
    let mut ordered_names = Vec::<String>::new();
    collect_names(root, &mut name_ids, &mut ordered_names);

    let mut dict = vec![0, char_size];
    for name in &ordered_names {
        let id = name_ids[name];
        let bytes = encode_chars(name, char_size);
        let len = u16::try_from(bytes.len()).expect("encoded name is too long");
        dict.push(15);
        dict.extend_from_slice(&id.to_le_bytes());
        dict.extend_from_slice(&len.to_le_bytes());
        dict.extend_from_slice(&bytes);
    }

    let mut doc = Vec::new();
    for header in header_raw {
        let bytes = encode_chars(header, char_size);
        doc.push(12);
        append_i32_len(&mut doc, bytes.len());
        doc.extend_from_slice(&bytes);
    }
    encode_node(root, char_size, &name_ids, &mut doc);

    Encoded { dict, doc }
}

fn collect_names(
    node: &XmlNode,
    ids: &mut HashMap<String, u16>,
    ordered: &mut Vec<String>,
) {
    fn add(name: &str, ids: &mut HashMap<String, u16>, ordered: &mut Vec<String>) {
        if !ids.contains_key(name) {
            let id = u16::try_from(ordered.len()).expect("too many distinct XML names");
            ids.insert(name.to_owned(), id);
            ordered.push(name.to_owned());
        }
    }

    add(&node.name, ids, ordered);
    for (name, _) in &node.attrs {
        add(name, ids, ordered);
    }
    for child in &node.children {
        collect_names(child, ids, ordered);
    }
}

fn encode_node(
    node: &XmlNode,
    char_size: u8,
    ids: &HashMap<String, u16>,
    output: &mut Vec<u8>,
) {
    let tag_id = ids[&node.name];
    output.push(1);
    output.extend_from_slice(&tag_id.to_le_bytes());

    for (name, value) in &node.attrs {
        let id = ids[name];
        match value {
            AttrValue::Str(value) => {
                output.push(3);
                output.extend_from_slice(&id.to_le_bytes());
                let bytes = encode_chars(value, char_size);
                append_i32_len(output, bytes.len());
                output.extend_from_slice(&bytes);
            }
            AttrValue::Int(value) => {
                output.push(4);
                output.extend_from_slice(&id.to_le_bytes());
                output.extend_from_slice(&value.to_le_bytes());
            }
            AttrValue::Bool(value) => {
                output.push(5);
                output.extend_from_slice(&id.to_le_bytes());
                output.push(u8::from(*value));
            }
            AttrValue::Long(value) => {
                output.push(6);
                output.extend_from_slice(&id.to_le_bytes());
                output.extend_from_slice(&value.to_le_bytes());
            }
            AttrValue::LongLong(value) => {
                output.push(7);
                output.extend_from_slice(&id.to_le_bytes());
                output.extend_from_slice(&value.to_le_bytes());
            }
            AttrValue::SizeT(value) => {
                output.push(8);
                output.extend_from_slice(&id.to_le_bytes());
                output.extend_from_slice(&value.to_le_bytes());
            }
            AttrValue::Float(value, digits) => {
                output.push(9);
                output.extend_from_slice(&id.to_le_bytes());
                output.extend_from_slice(&value.to_le_bytes());
                output.extend_from_slice(&digits.to_le_bytes());
            }
            AttrValue::Double(value, digits) => {
                output.push(10);
                output.extend_from_slice(&id.to_le_bytes());
                output.extend_from_slice(&value.to_le_bytes());
                output.extend_from_slice(&digits.to_le_bytes());
            }
            AttrValue::Blob(value) => {
                output.push(16);
                output.extend_from_slice(&id.to_le_bytes());
                append_i32_len(output, value.len());
                output.extend_from_slice(value);
            }
        }
    }

    if !node.text.is_empty() {
        output.push(11);
        let bytes = encode_chars(&node.text, char_size);
        append_i32_len(output, bytes.len());
        output.extend_from_slice(&bytes);
    }

    for child in &node.children {
        encode_node(child, char_size, ids, output);
    }

    output.push(2);
    output.extend_from_slice(&tag_id.to_le_bytes());
}

fn encode_chars(value: &str, char_size: u8) -> Vec<u8> {
    match char_size {
        1 => value.as_bytes().to_vec(),
        2 => value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
        4 => value
            .chars()
            .flat_map(|ch| u32::from(ch).to_le_bytes())
            .collect(),
        other => panic!("char_size must be 1, 2, or 4, got {other}"),
    }
}

fn append_i32_len(output: &mut Vec<u8>, len: usize) {
    let len = i32::try_from(len).expect("encoded field is too long");
    output.extend_from_slice(&len.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> XmlNode {
        let child = XmlNode::new("child")
            .attr("label", AttrValue::Str("hé𝄞".to_owned()))
            .attr("integer", AttrValue::Int(-123))
            .attr("boolean", AttrValue::Bool(true))
            .attr("long", AttrValue::Long(456))
            .attr("longlong", AttrValue::LongLong(-9_876_543_210))
            .attr("size", AttrValue::SizeT(4_000_000_000))
            .attr("float", AttrValue::Float(1.25, 3))
            .attr("double", AttrValue::Double(-2.5, 7))
            .attr("blob", AttrValue::Blob(vec![0, 1, 0xfe, 0xff]));
        let mut root = XmlNode::new("project").child(child);
        root.text = "text & content".to_owned();
        root
    }

    #[test]
    fn roundtrip_all_char_sizes_and_attribute_types() {
        let expected = sample_tree();
        for char_size in [1, 2, 4] {
            let encoded = encode(&expected, char_size, &["header"]);
            let decoded = decode(&encoded.dict, &encoded.doc).unwrap();
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn push_pop_scopes_restore_name_map() {
        let mut dict = vec![0, 1];
        for (id, name) in [(0u16, "root")] {
            dict.push(15);
            dict.extend_from_slice(&id.to_le_bytes());
            dict.extend_from_slice(&(name.len() as u16).to_le_bytes());
            dict.extend_from_slice(name.as_bytes());
        }

        let mut doc = vec![1, 0, 0, 13, 15, 1, 0, 5, 0];
        doc.extend_from_slice(b"inner");
        doc.extend_from_slice(&[1, 1, 0, 2, 1, 0, 14, 2, 0, 0]);

        let decoded = decode(&dict, &doc).unwrap();
        assert_eq!(decoded.name, "root");
        assert_eq!(decoded.children.len(), 1);
        assert_eq!(decoded.children[0].name, "inner");
    }

    #[test]
    fn truncated_input_returns_error() {
        let dict = [0, 1, 15];
        assert!(matches!(
            decode(&dict, &[]),
            Err(BinXmlError::Truncated(_))
        ));
    }

    #[test]
    fn unknown_type_returns_error() {
        let dict = [0, 1];
        let doc = [99];
        assert!(matches!(
            decode(&dict, &doc),
            Err(BinXmlError::UnknownType(99, 2))
        ));
    }

    #[test]
    fn accessors_and_xml_escaping() {
        let node = XmlNode::new("item")
            .attr("value", AttrValue::Str("1.5".to_owned()))
            .attr("title", AttrValue::Str("a&\"b".to_owned()));
        assert_eq!(node.f64("value"), Some(1.5));
        assert_eq!(
            node.to_xml_string(),
            "<item value=\"1.5\" title=\"a&amp;&quot;b\"/>\n"
        );
        assert_eq!(AttrValue::Int(-2).as_bool(), Some(true));
    }
}
