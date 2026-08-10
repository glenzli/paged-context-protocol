mod contract;
mod registration;
mod service;
mod snapshot;

pub use registration::ObserverConfig;
pub use service::ObserverService;

#[cfg(all(test, target_os = "macos"))]
pub(crate) use registration::canonical_runtime_root_for_test;

#[cfg(test)]
mod tests;
