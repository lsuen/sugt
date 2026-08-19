//! 平台重构测试套件（dev-platform-module 分支专用）
//!
//! 测试 `platform` 模块及各业务层的跨平台一致性。
//! 平台特有测试用 `#[cfg(windows)]` / `#[cfg(target_os = "macos")]` 隔离。

#[cfg(test)]
mod common_tests;
#[cfg(test)]
mod clients_tests;
#[cfg(windows)]
#[cfg(test)]
mod windows_tests;