/// Internal protocol version. Bump this when daemon/client wire format changes.
pub const PROTOCOL_VERSION: u32 = 1;

pub mod agent;
pub mod config;
pub mod mcp;
pub mod protocol;
pub mod skills;
pub mod tasks;
pub mod tool_adapter;
