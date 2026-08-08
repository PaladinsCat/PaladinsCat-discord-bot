//! Image generation module — mirrors TS Puppeteer pipeline via CDP WebSocket.

mod asset_catalog;
mod cdp_client;
mod match_renderer;
mod render_queue;
mod render_service;
mod template;

pub use asset_catalog::AssetCatalog;
pub use cdp_client::CdpClient;
pub use match_renderer::{MatchRenderer, MatchRendererConfig};
pub use render_queue::{BoundedWorkQueue, DurationMetrics, QueueFullError, QueueSnapshot};
pub use render_service::{ImageService, ImageServiceConfig};
pub use template::{TemplateConfig, TemplateEngine};
