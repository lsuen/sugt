//! `platform::common` 模块测试（跨平台通用逻辑）。

use sugt_lib::platform::common;

#[test]
fn finds_common_shell_on_path() {
    // Windows 上一定有 cmd/where；Unix 上一定有 sh/bash
    #[cfg(windows)]
    assert!(
        common::find_command_on_path("cmd").is_some()
            || common::find_command_on_path("where").is_some()
    );
    #[cfg(not(windows))]
    assert!(
        common::find_command_on_path("sh").is_some()
            || common::find_command_on_path("bash").is_some()
    );
}

#[test]
fn empty_command_returns_none() {
    assert!(common::find_command_on_path("").is_none());
}

#[test]
fn nonexistant_command_returns_none() {
    assert!(common::find_command_on_path("does_not_exist_xyz").is_none());
}

#[test]
fn finds_program_with_known_path() {
    let cmd = common::find_command_on_path("git");
    // 不要求一定存在（开发机可能没装 git），但逻辑不崩溃
    let _ = cmd;
}