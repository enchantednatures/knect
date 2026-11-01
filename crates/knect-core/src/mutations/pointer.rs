//! JSON Pointer (RFC 6901) encoding helpers shared by the mutation modules.

/// Escapes one reference token.
pub(crate) fn escape_segment(segment: &str) -> String {
    let mut escaped = String::with_capacity(segment.len());
    for character in segment.chars() {
        match character {
            '~' => escaped.push_str("~0"),
            '/' => escaped.push_str("~1"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Appends an escaped reference token to a JSON Pointer.
pub(crate) fn join(parent: &str, segment: &str) -> String {
    let mut pointer = String::with_capacity(parent.len() + segment.len() + 1);
    pointer.push_str(parent);
    pointer.push('/');
    pointer.push_str(&escape_segment(segment));
    pointer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_are_escaped_once() {
        assert_eq!(join("", "plain"), "/plain");
        assert_eq!(join("/a", "b/c"), "/a/b~1c");
        assert_eq!(join("/a", "b~c"), "/a/b~0c");
        assert_eq!(join("", "a~/b"), "/a~0~1b");
    }
}
