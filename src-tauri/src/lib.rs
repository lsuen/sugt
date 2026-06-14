pub mod anthropic_adapter;
pub mod app;
pub mod autostart;
pub mod clients;
pub mod commands;
pub mod config;
pub mod error_hint;
pub mod gateway;
pub mod gateway_stats;
pub mod logging;
pub mod model;
pub mod tray;
pub mod product;
pub mod provider_catalog;
pub mod trial;

pub use app::run;
