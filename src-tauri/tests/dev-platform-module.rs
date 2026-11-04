//! 平台重构测试套件（dev-platform-module 分支专用）
//!
//! 测试 `platform` 模块及各业务层的跨平台一致性。
//! 平台特有测试用 `#[cfg(windows)]` / `#[cfg(target_os = "macos")]` 隔离。

// 子模块文件位于同名目录内，必须显式指定路径，否则 rustc 会去 tests/ 根目录查找。
#[cfg(test)]
#[path = "dev-platform-module/common_tests.rs"]
mod common_tests;
#[cfg(test)]
#[path = "dev-platform-module/clients_tests.rs"]
mod clients_tests;
#[cfg(windows)]
#[cfg(test)]
#[path = "dev-platform-module/windows_tests.rs"]
mod windows_tests;