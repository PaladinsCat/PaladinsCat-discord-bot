//! Expose local assets, CDP transport, templates, queue, and image-service building blocks.
//!
//! MatchRenderer owns Chromium and HTML rendering; ImageService adds cache and recovery.
//! The bounded queue controls admission and shared results without owning backend data.
//! refs: doc: documents/05-operations/runbooks/discord-bot.md

mod asset_catalog;
mod cdp_client;
mod match_renderer;
mod render_queue;
mod render_service;
mod template;

pub use match_renderer::{MatchRenderer, MatchRendererConfig};
pub use render_service::{ImageService, ImageServiceConfig};
pub use template::{TemplateConfig, TemplateEngine};
