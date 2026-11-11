//! Live adapters. Each seam trait the lower crates define (`fsv_views::Source`,
//! `fsv_flows::meta::MetaSource`, `fsv_flows::change::ChangeWriter`,
//! `fsv_formui::FormBackend`, `fsv_tui::Backend` / `Pages`) gets one
//! implementation here over `fsv_core::Client` and `fsv_api`.
