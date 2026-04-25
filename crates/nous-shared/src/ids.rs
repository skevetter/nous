use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = std::convert::Infallible;

            fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
                Ok(Self(s.to_owned()))
            }
        }
    };
}

define_id!(SessionId);
define_id!(TraceId);
define_id!(SpanId);
define_id!(MemoryId);

impl Default for MemoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryId {
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }
}
