//! SkillRank core: registry client, content hashing, lockfile, install, stack
//! detection, and skill-surface discovery. This crate is dependency-light and
//! agent-agnostic so it can be embedded by the `skillrank` CLI and by BuildBetter
//! ZeroShot alike.

pub mod client;
pub mod config;
pub mod hash;
pub mod install;
pub mod lockfile;
pub mod repo;
pub mod runner;
pub mod skills;
pub mod stack;
pub mod types;

pub use client::{Client, ClientError, SearchOptions, PATH_PREFIX};
pub use hash::{compute_content_hash, hashes_equal, split_ref};
pub use install::{
    list_installed, safe_scan_tier, uninstall, InstallOptions, InstallResult, InstalledSkill,
};
pub use lockfile::{LockEntry, Lockfile};
pub use repo::{repo_root, resolve_surface};
pub use stack::{detect_stack, DetectedStack};
pub use types::*;
