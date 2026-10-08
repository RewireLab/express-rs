//! Path pattern matching for routes.
//!
//! Supports Express 5 path syntax:
//! - Static segments: `/users`
//! - Parameters: `/:id` — matches a single path segment
//! - Wildcards: `/*splat` — matches one or more segments
//! - Optional groups: `{/:op}` — matches zero or more segments
//! - Optional suffixes: `user{s}` — matches zero or one suffix
//!
//! Query strings are never part of the path pattern.

use std::collections::HashMap;

/// A parsed path pattern.
#[derive(Debug, Clone)]
pub struct PathPattern {
    segments: Vec<Segment>,
    case_sensitive: bool,
}

#[derive(Debug, Clone)]
enum Segment {
    Static(String),
    Param(String),
    Wildcard(String),
    Optional(Vec<Segment>),
}

/// Cursor over the request path, tracking position both by segment
/// index and by byte offset within the current segment (needed for
/// optional suffixes such as `user{s}` and delimiters such as `.:ext`).
#[derive(Clone)]
struct Cursor<'a> {
    segs: Vec<&'a str>,
    i: usize,
    off: usize,
}

impl<'a> Cursor<'a> {
    fn new(path: &'a str) -> Self {
        Self {
            segs: path.split('/').filter(|s| !s.is_empty()).collect(),
            i: 0,
            off: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.i >= self.segs.len()
    }

    /// True when sitting at the start of a segment (or the end of the path).
    fn at_boundary(&self) -> bool {
        self.off == 0
    }

    /// Remaining text of the current segment.
    fn rest(&self) -> &'a str {
        if self.i >= self.segs.len() {
            ""
        } else {
            &self.segs[self.i][self.off..]
        }
    }

    fn advance_segment(&mut self) {
        self.i += 1;
        self.off = 0;
    }

    /// Byte offsets just after each character of `rest`, plus the end.
    /// Index 0 is never yielded, so a param always captures at least
    /// one character.
    fn split_points(rest: &str) -> Vec<usize> {
        let mut points: Vec<usize> = rest.char_indices().skip(1).map(|(i, _)| i).collect();
        points.push(rest.len());
        points
    }
}

impl PathPattern {
    /// Parse a path string into a pattern.
    pub fn new(path: &str, case_sensitive: bool, _strict: bool) -> Self {
        Self {
            segments: parse(path),
            case_sensitive,
        }
    }

    /// Match a request path for an exact route match.
    ///
    /// Returns `Some(params)` if the whole path matches, `None` otherwise.
    /// A trailing slash on the request path is tolerated.
    ///
    /// Optional groups are resolved by first attempting a variant of the
    /// pattern in which every group is required. That makes
    /// `/:name{.:format}` prefer binding `name` and `format` on
    /// `/foo.json` instead of greedily swallowing the dot into `name`.
    pub fn match_path(&self, path: &str) -> Option<HashMap<String, String>> {
        let mut cur = Cursor::new(path);
        let mut params = HashMap::new();

        if self.match_from(&mandatory(&self.segments), 0, &mut cur, &mut params) && cur.at_end() {
            return Some(params);
        }

        let mut cur = Cursor::new(path);
        let mut params = HashMap::new();
        if self.match_from(&self.segments, 0, &mut cur, &mut params) && cur.at_end() {
            return Some(params);
        }

        None
    }

    /// Match a request path for a middleware mount.
    ///
    /// Like [`match_path`](Self::match_path) but allows extra trailing
    /// segments, so `/api` matches `/api/users/123`.
    pub fn match_prefix(&self, path: &str) -> Option<HashMap<String, String>> {
        let mut cur = Cursor::new(path);
        let mut params = HashMap::new();

        if self.match_from(&mandatory(&self.segments), 0, &mut cur, &mut params)
            && cur.at_boundary()
        {
            return Some(params);
        }

        let mut cur = Cursor::new(path);
        let mut params = HashMap::new();
        if self.match_from(&self.segments, 0, &mut cur, &mut params) && cur.at_boundary() {
            return Some(params);
        }

        None
    }

    /// Match `atoms[idx..]` against the cursor, backtracking where a
    /// wildcard or parameter could consume a variable amount of input.
    fn match_from(
        &self,
        atoms: &[Segment],
        idx: usize,
        cur: &mut Cursor<'_>,
        params: &mut HashMap<String, String>,
    ) -> bool {
        if idx >= atoms.len() {
            return true;
        }

        match &atoms[idx] {
            Segment::Static(s) => {
                if cur.at_end() {
                    return false;
                }
                let rest = cur.rest();
                if rest.len() < s.len() {
                    return false;
                }
                let head = &rest[..s.len()];
                let hit = if self.case_sensitive {
                    head == s.as_str()
                } else {
                    head.eq_ignore_ascii_case(s)
                };
                if !hit {
                    return false;
                }

                let mut probe = cur.clone();
                probe.off += s.len();
                if probe.off >= probe.segs[probe.i].len() {
                    probe.advance_segment();
                }

                if self.match_from(atoms, idx + 1, &mut probe, params) {
                    *cur = probe;
                    return true;
                }
                false
            }
            Segment::Param(name) => {
                if cur.at_end() {
                    return false;
                }
                let rest = cur.rest();

                // A param may stop mid-segment only when something other
                // than another param/wildcard follows it; otherwise it
                // must consume the whole segment so that `/:id/:op` can
                // never be satisfied by splitting one segment in two.
                let must_fill_segment = matches!(
                    atoms.get(idx + 1),
                    None | Some(Segment::Param(_)) | Some(Segment::Wildcard(_))
                );

                // Try the longest capture first so a bare param consumes
                // its whole segment; shorter splits are tried on backtrack
                // which is what binds a following delimiter (e.g. `.:ext`).
                let mut splits = if must_fill_segment {
                    vec![rest.len()]
                } else {
                    let mut s = Cursor::split_points(rest);
                    s.reverse();
                    s
                };

                splits.dedup();
                for split in splits {
                    let value = &rest[..split];
                    let mut probe = cur.clone();
                    probe.off += split;
                    if probe.off >= probe.segs[probe.i].len() {
                        probe.advance_segment();
                    }

                    let mut scratch = params.clone();
                    scratch.insert(name.clone(), decode(value));

                    if self.match_from(atoms, idx + 1, &mut probe, &mut scratch) {
                        *cur = probe;
                        *params = scratch;
                        return true;
                    }
                }
                false
            }
            Segment::Wildcard(name) => {
                if cur.at_end() {
                    return false;
                }

                // Try capturing as many segments as possible first,
                // falling back to fewer on backtrack.
                let first = cur.rest().to_string();
                let total = cur.segs.len() - cur.i;
                for take in (1..=total).rev() {
                    let mut parts = vec![first.clone()];
                    parts.extend(
                        cur.segs[cur.i + 1..cur.i + take]
                            .iter()
                            .map(|s| s.to_string()),
                    );

                    let mut probe = cur.clone();
                    probe.i += take;
                    probe.off = 0;

                    let mut scratch = params.clone();
                    scratch.insert(name.clone(), parts.join("/"));

                    if self.match_from(atoms, idx + 1, &mut probe, &mut scratch) {
                        *cur = probe;
                        *params = scratch;
                        return true;
                    }
                }
                false
            }
            Segment::Optional(inner) => {
                // Try the optional group; on failure, skip it.
                let mut probe = cur.clone();
                let mut scratch = params.clone();
                if self.match_from(inner, 0, &mut probe, &mut scratch)
                    && self.match_from(atoms, idx + 1, &mut probe, &mut scratch)
                {
                    *cur = probe;
                    *params = scratch;
                    return true;
                }

                self.match_from(atoms, idx + 1, cur, params)
            }
        }
    }
}

/// Produce a variant of `atoms` in which every optional group is
/// spliced in as a required group. Used to resolve optional groups in
/// preference to skipping them.
fn mandatory(atoms: &[Segment]) -> Vec<Segment> {
    let mut out = Vec::new();
    for atom in atoms {
        match atom {
            Segment::Optional(inner) => out.extend(mandatory(inner)),
            other => out.push(other.clone()),
        }
    }
    out
}

/// Parse a path string into a flat list of segments.
fn parse(path: &str) -> Vec<Segment> {
    let chars: Vec<char> = path.chars().collect();
    let mut pos = 0;
    let mut segments = Vec::new();

    while pos < chars.len() {
        match chars[pos] {
            '/' => pos += 1,
            ':' => {
                pos += 1;
                let name = read_name(&chars, &mut pos);
                segments.push(Segment::Param(name));
            }
            '*' => {
                pos += 1;
                let name = read_name(&chars, &mut pos);
                segments.push(Segment::Wildcard(name));
            }
            '{' => {
                pos += 1;
                let inner = read_braced(&chars, &mut pos);
                segments.push(Segment::Optional(parse(&inner)));
            }
            '\\' => {
                // Escaped literal character.
                pos += 1;
                if pos < chars.len() {
                    let lit = chars[pos].to_string();
                    pos += 1;
                    segments.push(Segment::Static(lit));
                }
            }
            _ => {
                let mut lit = String::new();
                while pos < chars.len() && !matches!(chars[pos], '/' | ':' | '*' | '{' | '\\') {
                    lit.push(chars[pos]);
                    pos += 1;
                }
                segments.push(Segment::Static(lit));
            }
        }
    }

    segments
}

/// Read a parameter or wildcard name (identifier characters).
fn read_name(chars: &[char], pos: &mut usize) -> String {
    let mut name = String::new();
    while *pos < chars.len() && (chars[*pos].is_alphanumeric() || chars[*pos] == '_') {
        name.push(chars[*pos]);
        *pos += 1;
    }
    name
}

/// Read the contents of a `{...}` group, honouring nesting.
fn read_braced(chars: &[char], pos: &mut usize) -> String {
    let mut inner = String::new();
    let mut depth = 1;

    while *pos < chars.len() {
        match chars[*pos] {
            '{' => {
                depth += 1;
                inner.push('{');
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    *pos += 1;
                    return inner;
                }
                inner.push('}');
            }
            c => inner.push(c),
        }
        *pos += 1;
    }

    inner
}

/// Percent-decode a path segment (e.g. `%20` -> ` `).
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(path: &str) -> PathPattern {
        PathPattern::new(path, false, false)
    }

    #[test]
    fn matches_static_exact() {
        let pat = p("/users");
        assert!(pat.match_path("/users").is_some());
        assert!(pat.match_path("/users/").is_some());
        assert!(pat.match_path("/user").is_none());
        assert!(pat.match_path("/users/1").is_none());
    }

    #[test]
    fn matches_param_single_segment() {
        let pat = p("/user/:id");
        assert_eq!(pat.match_path("/user/42").unwrap()["id"], "42");
        assert!(pat.match_path("/user").is_none());
        assert!(pat.match_path("/user/1/2").is_none());
    }

    #[test]
    fn decodes_percent_encoded_param() {
        let pat = p("/user/:name");
        assert_eq!(
            pat.match_path("/user/foo%20bar").unwrap()["name"],
            "foo bar"
        );
    }

    #[test]
    fn matches_optional_group_present_and_absent() {
        let pat = p("/user/:id{/:op}");

        let absent = pat.match_path("/user/1").unwrap();
        assert_eq!(absent["id"], "1");
        assert!(!absent.contains_key("op"));

        let present = pat.match_path("/user/1/edit").unwrap();
        assert_eq!(present["id"], "1");
        assert_eq!(present["op"], "edit");
    }

    #[test]
    fn matches_optional_suffix() {
        let pat = p("/user{s}");

        assert!(pat.match_path("/users").unwrap().is_empty());
        assert!(pat.match_path("/user").unwrap().is_empty());
        assert!(pat.match_path("/users/x").is_none());
    }

    #[test]
    fn optional_suffix_before_params() {
        let pat = p("/user{s}/:user/:op");

        assert_eq!(pat.match_path("/user/tj/edit").unwrap()["user"], "tj");
        assert_eq!(pat.match_path("/users/tj/edit").unwrap()["user"], "tj");
    }

    #[test]
    fn wildcard_captures_remaining_segments() {
        let pat = p("/files/*splat");

        assert_eq!(pat.match_path("/files/a/b/c").unwrap()["splat"], "a/b/c");
        assert_eq!(pat.match_path("/files/x.txt").unwrap()["splat"], "x.txt");
        assert!(pat.match_path("/files").is_none());
    }

    #[test]
    fn wildcard_requires_at_least_one_segment() {
        // Express 5 omits an unmatched optional wildcard entirely.
        let pat = p("/user{/*splat}");
        let m = pat.match_path("/user").unwrap();
        assert!(!m.contains_key("splat"));
    }

    #[test]
    fn literal_dot_splits_params() {
        let pat = p("/:name.:format");
        let m = pat.match_path("/foo.json").unwrap();
        assert_eq!(m["name"], "foo");
        assert_eq!(m["format"], "json");
        assert!(pat.match_path("/foo").is_none());
    }

    #[test]
    fn optional_format_suffix() {
        let pat = p("/:name{.:format}");
        assert_eq!(pat.match_path("/foo").unwrap()["name"], "foo");
        let m = pat.match_path("/foo.json").unwrap();
        assert_eq!(m["name"], "foo");
        assert_eq!(m["format"], "json");
    }

    #[test]
    fn escaped_characters_are_literal() {
        let pat = p("/:user\\(:op\\)");
        let m = pat.match_path("/tj(edit)").unwrap();
        assert_eq!(m["user"], "tj");
        assert_eq!(m["op"], "edit");
    }

    #[test]
    fn prefix_match_allows_trailing_segments() {
        let pat = p("/api");
        assert!(pat.match_prefix("/api").is_some());
        assert!(pat.match_prefix("/api/users").is_some());
        assert!(pat.match_prefix("/api/users/1").is_some());
        assert!(pat.match_prefix("/other").is_none());
    }

    #[test]
    fn prefix_match_stops_on_segment_boundary() {
        let pat = p("/api");
        assert!(pat.match_prefix("/apixyz").is_none());
    }

    #[test]
    fn root_pattern_matches_only_root() {
        let pat = p("/");
        assert!(pat.match_path("/").is_some());
        assert!(pat.match_path("/anything").is_none());
    }

    #[test]
    fn case_insensitive_by_default() {
        let pat = p("/User");
        assert!(pat.match_path("/user").is_some());
        assert!(pat.match_path("/USER").is_some());
    }

    #[test]
    fn case_sensitive_when_enabled() {
        let pat = PathPattern::new("/uSer", true, false);
        assert!(pat.match_path("/uSer").is_some());
        assert!(pat.match_path("/user").is_none());
    }

    #[test]
    fn params_adjacent_to_static_text() {
        let pat = p("/api/v:version/users");
        let m = pat.match_path("/api/v2/users").unwrap();
        assert_eq!(m["version"], "2");
    }
}
