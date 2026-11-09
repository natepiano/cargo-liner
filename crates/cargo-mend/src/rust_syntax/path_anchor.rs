#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathAnchor {
    Crate,
    Super,
    SelfMod,
    SelfType,
    Name,
}

impl PathAnchor {
    pub(crate) fn first(segments: &[String]) -> Option<Self> {
        segments.first().map(|segment| Self::from(segment.as_str()))
    }
}

impl From<&str> for PathAnchor {
    fn from(segment: &str) -> Self {
        match segment {
            "crate" => Self::Crate,
            "super" => Self::Super,
            "self" => Self::SelfMod,
            "Self" => Self::SelfType,
            _ => Self::Name,
        }
    }
}

pub(crate) fn leading_super_count(segments: &[String]) -> usize {
    segments
        .iter()
        .take_while(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::Super)
        .count()
}

pub(crate) fn trim_leading_self(segments: &[String]) -> &[String] {
    match PathAnchor::first(segments) {
        Some(PathAnchor::SelfMod) => &segments[1..],
        _ => segments,
    }
}
