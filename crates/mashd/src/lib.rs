include!(concat!(env!("OUT_DIR"), "/protocol_version.rs"));

pub mod agent;
pub mod config;
pub mod mcp;
pub mod protocol;
pub mod skills;
pub mod tasks;
pub mod tool_adapter;
