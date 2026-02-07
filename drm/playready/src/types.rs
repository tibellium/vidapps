use core::fmt;
use core::str::FromStr;

use drm_core::{
    ParseError,
    utils::{eq_ignore_ascii_case, trim_ascii},
};

/**
    PlayReady security level as stored in BCert BasicInfo.

    Known values:
    - **SL150** — test/development (lowest security)
    - **SL2000** — software-based with hardware root of trust
    - **SL3000** — software-only (most common for consumer devices)
*/
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecurityLevel {
    SL150,
    SL2000,
    SL3000,
    Unknown(u32),
}

impl SecurityLevel {
    pub const fn from_u32(u: u32) -> Self {
        match u {
            150 => Self::SL150,
            2000 => Self::SL2000,
            3000 => Self::SL3000,
            _ => Self::Unknown(u),
        }
    }

    pub const fn to_u32(self) -> u32 {
        match self {
            Self::SL150 => 150,
            Self::SL2000 => 2000,
            Self::SL3000 => 3000,
            Self::Unknown(u) => u,
        }
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            3 if eq_ignore_ascii_case(name, b"150") => Some(Self::SL150),
            4 if eq_ignore_ascii_case(name, b"2000") => Some(Self::SL2000),
            4 if eq_ignore_ascii_case(name, b"3000") => Some(Self::SL3000),
            5 if eq_ignore_ascii_case(name, b"sl150") => Some(Self::SL150),
            6 if eq_ignore_ascii_case(name, b"sl2000") => Some(Self::SL2000),
            6 if eq_ignore_ascii_case(name, b"sl3000") => Some(Self::SL3000),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::SL150 => "SL150",
            Self::SL2000 => "SL2000",
            Self::SL3000 => "SL3000",
            Self::Unknown(_) => "Unknown",
        }
    }
}

impl fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(u) => write!(f, "SL{u}"),
            _ => f.write_str(self.to_name()),
        }
    }
}

impl FromStr for SecurityLevel {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "security level",
            value: s.to_owned(),
        })
    }
}

impl From<u32> for SecurityLevel {
    fn from(u: u32) -> Self {
        Self::from_u32(u)
    }
}

impl From<SecurityLevel> for u32 {
    fn from(sl: SecurityLevel) -> Self {
        sl.to_u32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_levels_round_trip() {
        for val in [150u32, 2000, 3000] {
            let sl = SecurityLevel::from_u32(val);
            assert_eq!(sl.to_u32(), val);
            assert!(!matches!(sl, SecurityLevel::Unknown(_)));
        }
    }

    #[test]
    fn unknown_level_preserved() {
        let sl = SecurityLevel::from_u32(9999);
        assert!(matches!(sl, SecurityLevel::Unknown(9999)));
        assert_eq!(sl.to_u32(), 9999);
    }

    #[test]
    fn display_known() {
        assert_eq!(SecurityLevel::SL150.to_string(), "SL150");
        assert_eq!(SecurityLevel::SL2000.to_string(), "SL2000");
        assert_eq!(SecurityLevel::SL3000.to_string(), "SL3000");
    }

    #[test]
    fn display_unknown() {
        assert_eq!(SecurityLevel::Unknown(42).to_string(), "SL42");
    }

    #[test]
    fn from_name_bare_numbers() {
        assert_eq!(SecurityLevel::from_name(b"150"), Some(SecurityLevel::SL150));
        assert_eq!(
            SecurityLevel::from_name(b"2000"),
            Some(SecurityLevel::SL2000)
        );
        assert_eq!(
            SecurityLevel::from_name(b"3000"),
            Some(SecurityLevel::SL3000)
        );
    }

    #[test]
    fn from_name_with_prefix() {
        assert_eq!(
            SecurityLevel::from_name(b"sl150"),
            Some(SecurityLevel::SL150)
        );
        assert_eq!(
            SecurityLevel::from_name(b"SL2000"),
            Some(SecurityLevel::SL2000)
        );
        assert_eq!(
            SecurityLevel::from_name(b"SL3000"),
            Some(SecurityLevel::SL3000)
        );
        assert_eq!(
            SecurityLevel::from_name(b"Sl3000"),
            Some(SecurityLevel::SL3000)
        );
    }

    #[test]
    fn from_name_rejects_bad() {
        assert_eq!(SecurityLevel::from_name(b""), None);
        assert_eq!(SecurityLevel::from_name(b"bad"), None);
        assert_eq!(SecurityLevel::from_name(b"SL9999"), None);
    }

    #[test]
    fn name_round_trip() {
        for sl in [
            SecurityLevel::SL150,
            SecurityLevel::SL2000,
            SecurityLevel::SL3000,
        ] {
            let name = sl.to_name();
            let parsed = SecurityLevel::from_name(name.as_bytes()).unwrap();
            assert_eq!(parsed, sl);
        }
    }

    #[test]
    fn from_str_works() {
        assert_eq!(
            "SL3000".parse::<SecurityLevel>().unwrap(),
            SecurityLevel::SL3000
        );
        assert_eq!(
            "150".parse::<SecurityLevel>().unwrap(),
            SecurityLevel::SL150
        );
        assert!("bad".parse::<SecurityLevel>().is_err());
    }

    #[test]
    fn u32_conversions() {
        let sl: SecurityLevel = 3000u32.into();
        assert_eq!(sl, SecurityLevel::SL3000);
        let val: u32 = sl.into();
        assert_eq!(val, 3000);
    }
}
