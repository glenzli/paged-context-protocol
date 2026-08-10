mod contract;
mod registration;
mod service;
mod snapshot;

pub use registration::ObserverConfig;
pub use service::ObserverService;

#[cfg(test)]
mod tests;
