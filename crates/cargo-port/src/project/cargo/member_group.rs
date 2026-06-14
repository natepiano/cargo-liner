use super::package::Package;

/// Members within a workspace organized into groups.
#[derive(Clone)]
pub(crate) enum MemberGroup {
    Named {
        name:    String,
        members: Vec<Package>,
    },
    Inline {
        members: Vec<Package>,
    },
}

impl MemberGroup {
    pub fn members(&self) -> &[Package] {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }

    pub const fn members_mut(&mut self) -> &mut Vec<Package> {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }

    pub fn group_name(&self) -> &str {
        match self {
            Self::Named { name, .. } => name,
            Self::Inline { .. } => "",
        }
    }

    pub const fn is_named(&self) -> bool { matches!(self, Self::Named { .. }) }

    pub const fn is_inline(&self) -> bool { matches!(self, Self::Inline { .. }) }

    pub fn into_members(self) -> Vec<Package> {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }
}
