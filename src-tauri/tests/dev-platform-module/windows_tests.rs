//! Windows 平台特有测试。
//!
//! 需要注册表的测试在此文件中，避免在 CI Unix runner 上失败。

#[cfg(windows)]
mod tests {
    use sugt_lib::platform::common;

    #[test]
    fn finds_executables_on_windows() {
        // Windows 上 cmd/where 必然存在
        assert!(common::find_command_on_path("cmd").is_some());
    }
}