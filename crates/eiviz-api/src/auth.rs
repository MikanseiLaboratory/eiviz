use std::path::PathBuf;

use subtle::ConstantTimeEq;

use crate::proto::Role as ProtoRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Read = 1,
    Operate = 2,
    Configure = 3,
    Admin = 4,
}

impl Role {
    pub fn from_proto(role: ProtoRole) -> Self {
        match role {
            ProtoRole::Read => Self::Read,
            ProtoRole::Operate => Self::Operate,
            ProtoRole::Configure => Self::Configure,
            ProtoRole::Admin => Self::Admin,
            ProtoRole::Unspecified => Self::Read,
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "operate" => Self::Operate,
            "configure" => Self::Configure,
            "admin" => Self::Admin,
            _ => Self::Read,
        }
    }

    pub fn allows(self, required: Role) -> bool {
        self as u8 >= required as u8
    }
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub token: String,
    pub require_auth: bool,
    pub max_role: Role,
}

impl AuthConfig {
    pub fn from_env() -> Self {
        let token = std::env::var("EIVIZ_API_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("EIVIZ_API_TOKEN_FILE")
                    .ok()
                    .and_then(|path| std::fs::read_to_string(PathBuf::from(path)).ok())
                    .map(|text| text.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_default();
        let require_auth = std::env::var("EIVIZ_API_REQUIRE_AUTH")
            .map(|value| value != "0")
            .unwrap_or(!token.is_empty());
        let max_role = std::env::var("EIVIZ_API_ROLE")
            .ok()
            .map(|value| Role::from_name(&value))
            .unwrap_or(if token.is_empty() {
                Role::Read
            } else {
                Role::Admin
            });
        Self {
            token,
            require_auth,
            max_role,
        }
    }

    pub fn granted_role(&self, requested: Role) -> Role {
        if requested < self.max_role {
            requested
        } else {
            self.max_role
        }
    }

    pub fn check(&self, presented: &str) -> bool {
        if !self.require_auth && self.token.is_empty() {
            return true;
        }
        if self.token.is_empty() {
            return false;
        }
        constant_eq(presented.as_bytes(), self.token.as_bytes())
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare_is_length_checked() {
        let auth = AuthConfig {
            token: "secret".into(),
            require_auth: true,
            max_role: Role::Admin,
        };
        assert!(auth.check("secret"));
        assert!(!auth.check("secre"));
        assert!(!auth.check("secret!"));
    }
}
