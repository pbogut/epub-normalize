use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
const PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
#[derive(Debug)]
pub struct Uri {
    pub path: String,
    pub query: String,
    pub fragment: String,
    pub scheme: String,
    pub external: bool,
}
impl Uri {
    pub fn parse(value: &str) -> Self {
        let value = value
            .trim_start_matches(|c: char| c <= ' ')
            .replace(['\n', '\r', '\t'], "");
        let (rest, fragment) = value.split_once('#').unwrap_or((&value, ""));
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let scheme = path
            .split_once(':')
            .filter(|(s, _)| {
                s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            })
            .map_or(String::new(), |(s, _)| s.to_lowercase());
        let external = !scheme.is_empty() || path.starts_with("//");
        Self {
            path: path.into(),
            query: query.into(),
            fragment: fragment.into(),
            scheme,
            external,
        }
    }
    pub fn dangerous(&self) -> bool {
        matches!(
            self.scheme.as_str(),
            "javascript" | "data" | "vbscript" | "file"
        )
    }
    pub fn with_path(&self, path: &str) -> String {
        let mut value = utf8_percent_encode(path, PATH).to_string();
        if !self.query.is_empty() {
            value.push('?');
            value.push_str(&self.query);
        }
        if !self.fragment.is_empty() {
            value.push('#');
            value.push_str(&self.fragment);
        }
        value
    }
}
pub fn decode(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}
