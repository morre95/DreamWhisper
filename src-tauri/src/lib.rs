pub mod archive;
pub mod autostart;
pub mod db;
pub mod images;
pub mod devices;
pub mod service;
pub mod types;
pub mod worker;

#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
pub use desktop::run;
