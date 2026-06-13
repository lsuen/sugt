use std::process::{Command, Output};

/// 后台执行子进程，Windows 下不弹出控制台窗口。
pub fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    hidden_command(program).args(args).output()
}
