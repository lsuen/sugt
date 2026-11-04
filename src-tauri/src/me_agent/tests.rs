//! `me_agent` 模块单元测试（由 me_agent.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn parses_bash_and_keeps_the_preface() {
    let action = parse_model_action("我先看一眼。\n```bash\nls -la\n```\n好。").unwrap();
    match action {
        ModelAction::Run { prose, command } => {
            assert!(prose.contains("我先看一眼"));
            assert!(prose.contains("好"));
            assert_eq!(command, "ls -la");
        }
        ModelAction::Say(_) => panic!("expected a command"),
    }
}

#[test]
fn plain_text_is_a_reply() {
    assert_eq!(
        parse_model_action("做完了").unwrap(),
        ModelAction::Say("做完了".to_string())
    );
}

#[test]
fn empty_reply_is_a_format_error() {
    assert!(parse_model_action("  ").is_err());
    assert!(parse_model_action("```bash\n\n```").is_err());
}

#[test]
fn only_the_first_fence_runs() {
    let action = parse_model_action("```bash\necho one\n```\n```bash\necho two\n```").unwrap();
    match action {
        ModelAction::Run { command, .. } => assert_eq!(command, "echo one"),
        ModelAction::Say(_) => panic!("expected a command"),
    }
}

#[test]
fn confirm_heuristic_pauses_writes_and_lets_reads_through() {
    assert!(!command_needs_confirm("ls -la"));
    assert!(!command_needs_confirm("echo hi"));
    assert!(!command_needs_confirm("pwd && ls"));
    assert!(!command_needs_confirm("git status"));
    assert!(!command_needs_confirm("sed -n '1p' readme"));
    assert!(!command_needs_confirm("echo \"a > b\""));
    assert!(!command_needs_confirm("ls # rm -rf /"));
    assert!(!command_needs_confirm("open tetris.html"));
    assert!(!command_needs_confirm("xdg-open ./game.html"));
    assert!(!command_needs_confirm("Invoke-Item .\\tetris.html"));
    assert!(!command_needs_confirm("cmd.exe /c start \"\" tetris.html"));
    assert!(command_needs_confirm("echo hi > notes.txt"));
    assert!(command_needs_confirm("rm -rf tmp"));
    assert!(command_needs_confirm("mkdir notes"));
    assert!(command_needs_confirm("ls && rm x"));
    assert!(command_needs_confirm("cat a | tee b"));
    assert!(command_needs_confirm("git commit -m \"x\""));
    assert!(command_needs_confirm("sed -i 's/a/b/' f"));
    assert!(command_needs_confirm("echo $(pwd)"));
}

#[test]
fn system_prompt_names_detected_host_os_and_open_recipe() {
    let dir = temp_dir("prompt-os");
    let prompt = system_prompt(true, &dir, &dir.join("projects"), &dir, "项目里");
    assert!(prompt.contains(host_os_label()));
    assert!(prompt.contains(open_file_hint()));
}

#[test]
fn stream_delta_reads_openai_content_and_text_parts() {
    let plain = json!({"choices":[{"delta":{"content":"你好"}}]});
    assert_eq!(extract_stream_delta(&plain).as_deref(), Some("你好"));
    let parts = json!({"choices":[{"delta":{"content":[
        {"type":"text","text":"Hi "},
        {"type":"text","text":"su"}
    ]}}]});
    assert_eq!(extract_stream_delta(&parts).as_deref(), Some("Hi su"));
    let role_only = json!({"choices":[{"delta":{"role":"assistant"}}]});
    assert!(extract_stream_delta(&role_only).is_none());
    let anth = json!({
        "type":"content_block_delta",
        "delta":{"type":"text_delta","text":"流"}
    });
    assert_eq!(extract_stream_delta(&anth).as_deref(), Some("流"));
}

#[tokio::test]
async fn auto_write_keeps_running_a_second_write_in_the_same_loop() {
    let dir = temp_dir("loop-auto-multi");
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_ask = calls.clone();
    let ask = move |_messages: Vec<Value>| {
        let n = calls_ask.fetch_add(1, Ordering::SeqCst);
        async move {
            match n {
                0 => Ok("```bash\nmkdir one\n```".to_string()),
                1 => Ok("```bash\nmkdir two\n```".to_string()),
                _ => Ok("两个都建好了".to_string()),
            }
        }
    };
    let result = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        LoopLimits::default(),
        true,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert!(result.pending.is_none());
    assert!(dir.join("one").is_dir());
    assert!(dir.join("two").is_dir());
    assert!(result.content.contains("两个"));
    assert!(calls.load(Ordering::SeqCst) >= 3);
}

#[test]
fn phrases_confirm_or_dismiss_only_when_exact() {
    assert!(matches!(intake("写吧。"), Intake::Confirm));
    assert!(matches!(intake("Go ahead"), Intake::Confirm));
    assert!(matches!(intake("先别写"), Intake::Dismiss));
    assert!(matches!(intake("写吧，但是换个名字"), Intake::Talk(_)));
}

#[test]
fn prune_shell_output_keeps_head_and_tail() {
    let short = "hello\n";
    let (kept, truncated) = prune_shell_output(short, 100);
    assert!(!truncated);
    assert_eq!(kept, short);

    let mut body = String::new();
    for i in 0..200 {
        body.push_str(&format!("LINE-{i:03}-ABCDEFGHIJKLMNOPQRSTUVWXYZ\n"));
    }
    let limit = 800;
    let (pruned, truncated) = prune_shell_output(&body, limit);
    assert!(truncated);
    assert!(pruned.chars().count() <= limit);
    assert!(pruned.starts_with("LINE-000-"));
    assert!(pruned.contains("LINE-199-"));
    assert!(pruned.contains("omitted"));
    assert!(pruned.contains("sed -n"));
    assert!(!pruned.contains("LINE-100-"));
}

#[test]
fn fake_tool_calls_are_not_treated_as_done_prose() {
    let fake = "好的。<|tool_call_start|>[write(path='/tmp/a.md', content='hi')]<|tool_call_end|>";
    assert!(looks_like_unsupported_tools(fake));
    assert!(parse_model_action(fake).is_err());
    assert!(matches!(
        parse_model_action("就这样吧，先不动文件。"),
        Ok(ModelAction::Say(_))
    ));
    assert!(matches!(
        parse_model_action("写一下\n```bash\necho hi > a.md\n```"),
        Ok(ModelAction::Run { .. })
    ));
}

#[test]
fn safety_only_replies_are_rejected() {
    let safety = "User Safety: unsafe\nResponse Safety: safe\nSafety Categories: Profanity";
    assert!(is_safety_only_reply(safety));
    assert!(parse_model_action(safety).is_err());
    let mixed = "User Safety: safe\n好的，我来写文件。\n```bash\ntouch a.txt\n```";
    assert!(matches!(
        parse_model_action(mixed),
        Ok(ModelAction::Run { .. })
    ));
}

#[test]
fn plan_only_replies_are_rejected() {
    assert!(looks_like_plan_without_command(
        "好的，按规范先读文档，再创建 txt。"
    ));
    assert!(parse_model_action("好的，按规范先读文档，再创建 txt。").is_err());
    assert!(looks_like_false_completion(
        "✅ 已完成！\nexit: 0\ntotal 0\ndrwxr-xr-x 1 me staff 0 ."
    ));
    assert!(parse_model_action("✅ 全部文件已准备妥当！").is_err());
    assert!(has_unclosed_fence(
        "先读再写：\n```bash\ncat AGENTS.md && cat > /Users/sunwenlong/Documen"
    ));
    assert!(parse_model_action(
        "先读再写：\n```bash\ncat AGENTS.md && cat > /Users/sunwenlong/Documen"
    )
    .is_err());
    assert!(matches!(
        parse_model_action("随便聊聊天气吧。"),
        Ok(ModelAction::Say(_))
    ));
    // 长计划（曾因 <160 字限制被误判成说完）必须催出代码块
    let long_plan = "用户要求写一个H5贪吃蛇小游戏并打开。我需要在当前工作目录 \
/Users/x/Library/Application Support/sugt/projects/snake-play 创建一个HTML文件，然后打开它。\n\n\
让我先写一个完整的贪吃蛇游戏HTML文件。游戏应该包含：\n- 得分\n\n\
I'll create a single self-contained HTML file.";
    assert!(looks_like_plan_without_command(long_plan));
    assert!(parse_model_action(long_plan).is_err());
}

#[test]
fn detect_mini_demo_recognizes_snake_and_tetris() {
    let snake = detect_mini_demo("帮我写个h5的贪吃蛇小游戏并打开").expect("snake");
    assert_eq!(snake.0, "snake.html");
    assert!(snake.1.contains("Snake"));
    let tetris = detect_mini_demo("写个俄罗斯方块 html 打开").expect("tetris");
    assert_eq!(tetris.0, "tetris.html");
    assert!(detect_mini_demo("今天天气怎么样").is_none());
}

#[test]
fn images_normalize_and_build_multimodal_user_content() {
    let ok = normalize_images(Some(vec![MeImage {
        media_type: "image/png".into(),
        data: "aGVsbG8=".into(),
    }]))
    .unwrap();
    assert_eq!(ok.len(), 1);
    assert!(normalize_images(Some(vec![MeImage {
        media_type: "image/svg+xml".into(),
        data: "aGVsbG8=".into(),
    }]))
    .is_err());
    let message = MeMessage {
        id: "1".into(),
        role: "user".into(),
        content: "看图".into(),
        images: ok,
        steps: Vec::new(),
        created_at: 0,
    };
    let content = user_model_content(&message, true).unwrap();
    let parts = content.as_array().unwrap();
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[1]["type"], "image_url");
    assert!(parts[1]["image_url"]["url"]
        .as_str()
        .unwrap()
        .starts_with("data:image/png;base64,"));
}

#[test]
fn prune_keeps_images_only_on_latest_user_turn() {
    let mut messages = vec![
        MeMessage {
            id: "u1".into(),
            role: "user".into(),
            content: "a".into(),
            images: vec![MeImage {
                media_type: "image/png".into(),
                data: "YQ==".into(),
            }],
            steps: Vec::new(),
            created_at: 1,
        },
        MeMessage {
            id: "a1".into(),
            role: "assistant".into(),
            content: "ok".into(),
            images: Vec::new(),
            steps: Vec::new(),
            created_at: 2,
        },
        MeMessage {
            id: "u2".into(),
            role: "user".into(),
            content: "b".into(),
            images: vec![MeImage {
                media_type: "image/png".into(),
                data: "Yg==".into(),
            }],
            steps: Vec::new(),
            created_at: 3,
        },
    ];
    prune_old_images(&mut messages);
    assert!(messages[0].images.is_empty());
    assert_eq!(messages[2].images.len(), 1);
}

#[test]
fn project_names_stay_a_single_path_segment() {
    assert!(sanitize_project_name("../config").is_err());
    assert!(sanitize_project_name("a/b").is_err());
    assert!(sanitize_project_name(".hidden").is_err());
    assert_eq!(sanitize_project_name(" 笔记 ").unwrap(), "笔记");
}

#[test]
fn project_work_dir_is_under_projects_and_config_is_the_parent() {
    let config = Path::new("/Users/me/Documents/.sugt");
    let project = resolve_work_dir(config, "project", "notes");
    let gateway = resolve_work_dir(config, "config", "notes");
    assert_eq!(project, config.join("projects").join("notes"));
    assert_eq!(gateway, config);
    assert_ne!(project, gateway);
    assert!(project.starts_with(projects_root(config)));
}

#[test]
fn prompt_names_both_directories_and_a_fresh_shell() {
    let config = Path::new("/tmp/sugt-home");
    let work = config.join("projects").join("default");
    let prompt = system_prompt(
        true,
        &work,
        &projects_root(config),
        config,
        "项目「default」里",
    );
    assert!(prompt.contains("/tmp/sugt-home/projects"));
    assert!(prompt.contains("config.toml"));
    assert!(prompt.contains("新的 shell"));
    let english = system_prompt(
        false,
        &work,
        &projects_root(config),
        config,
        "project \"default\"",
    );
    assert!(english.contains("new shell"));
}

#[test]
fn shell_runs_in_the_given_directory() {
    let dir = temp_dir("echo");
    fs::write(dir.join("marker.txt"), "here").unwrap();
    let outcome = run_shell(&dir, "pwd && cat marker.txt", Duration::from_secs(5), 1000);
    assert_eq!(outcome.exit_code, Some(0), "{}", outcome.output);
    assert!(outcome.output.contains("here"), "{}", outcome.output);
    let shown = outcome.output.lines().next().unwrap().trim();
    assert_eq!(
        Path::new(shown).canonicalize().unwrap(),
        dir.canonicalize().unwrap()
    );
}

#[test]
fn shell_times_out_and_stops() {
    let dir = temp_dir("sleep");
    let outcome = run_shell(&dir, "sleep 5", Duration::from_millis(300), 1000);
    assert!(outcome.timed_out);
}

#[tokio::test]
async fn loop_runs_a_read_then_stops_on_prose() {
    let dir = temp_dir("loop-read");
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_ask = calls.clone();
    let ask = move |_messages: Vec<Value>| {
        let n = calls_ask.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Ok("```bash\necho from-su\n```".to_string())
            } else {
                Ok("看到了".to_string())
            }
        }
    };
    let result = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        LoopLimits::default(),
        false,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "看到了");
    assert!(result.pending.is_none());
    assert_eq!(result.steps.len(), 1);
    assert!(result.steps[0].output.contains("from-su"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn loop_holds_a_write_until_a_later_confirm_runs_it() {
    let dir = temp_dir("loop-write");
    let ask = |_messages: Vec<Value>| async {
        Ok("建一个目录\n```bash\nmkdir subdir\n```".to_string())
    };
    let held = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        LoopLimits::default(),
        false,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert!(held.pending.is_some());
    assert!(!dir.join("subdir").exists());
    assert!(held.content.contains("建一个目录"));

    let auto_calls = Arc::new(AtomicUsize::new(0));
    let auto_calls_ask = auto_calls.clone();
    let ask_auto = move |_messages: Vec<Value>| {
        let n = auto_calls_ask.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Ok("自动写\n```bash\nmkdir auto_subdir\n```".to_string())
            } else {
                Ok("目录已经建上了。".to_string())
            }
        }
    };
    let autoed = run_loop(
        ask_auto,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        LoopLimits::default(),
        true,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert!(autoed.pending.is_none());
    assert!(dir.join("auto_subdir").is_dir());
    // 写入成功后会多一步「写后核对」
    assert!(autoed.steps.len() >= 2);
    assert!(autoed.steps.iter().any(|s| s.note.contains("写后核对")));
    assert!(autoed.content.contains("目录"));

    let command = held.pending.unwrap().command;
    let ask = |_messages: Vec<Value>| async { Ok("建好了".to_string()) };
    let done = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        Some(command),
        "建一个目录",
        "system",
        LoopLimits::default(),
        false,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert!(dir.join("subdir").is_dir());
    assert_eq!(done.content, "建好了");
    assert!(done.steps.len() >= 2);
    assert!(done.steps.iter().any(|s| s.note.contains("写后核对")));
    assert!(done.pending.is_none());
}

#[test]
fn extract_write_paths_finds_redirects_and_mkdir() {
    let paths = extract_write_paths("echo hi > notes.txt && mkdir -p docs/out");
    assert!(paths.iter().any(|p| p == "notes.txt"));
    assert!(paths.iter().any(|p| p == "docs/out"));
    let tee = extract_write_paths("cat <<'EOF' > story.md\nhello\nEOF");
    assert!(tee.iter().any(|p| p == "story.md"));
}

#[tokio::test]
async fn empty_model_text_is_retried_then_accepted() {
    let dir = temp_dir("loop-format");
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_ask = calls.clone();
    let ask = move |_messages: Vec<Value>| {
        let n = calls_ask.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Ok("   ".to_string())
            } else {
                Ok("在的".to_string())
            }
        }
    };
    let result = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        LoopLimits::default(),
        false,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "在的");
    assert!(result.steps.is_empty());
}

#[tokio::test]
async fn step_limit_stops_a_loop_that_never_talks() {
    let dir = temp_dir("loop-limit");
    let ask = |_messages: Vec<Value>| async { Ok("```bash\necho again\n```".to_string()) };
    let mut limits = LoopLimits::default();
    limits.steps = 2;
    let result = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        limits,
        false,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert_eq!(result.steps.len(), 2);
    assert!(result.content.contains("步数"));
    assert!(result.pending.is_none());
}

#[test]
fn session_roundtrip_keeps_pending_away_from_the_config_file_name() {
    let dir = temp_dir("session");
    let mut session = MeSession::default();
    session.project = "笔记".to_string();
    session.pending = Some(MePending {
        command: "printf 'hi' > readme.txt".to_string(),
        prose: "写到项目里".to_string(),
    });
    push_message(&mut session, "user", "写个说明".to_string(), Vec::new());
    save_session(&dir, &session).unwrap();
    let loaded = load_session(&dir);
    assert_eq!(loaded.project, "笔记");
    assert_eq!(loaded.pending.unwrap().prose, "写到项目里");
    assert!(chat_path(&dir, "project", "笔记").starts_with(chats_dir(&dir)));
    assert!(!chat_path(&dir, "project", "笔记").ends_with("config.toml"));
    let work = resolve_work_dir(&dir, &loaded.mode, &loaded.project);
    assert_eq!(work, dir.join("projects").join("笔记"));
    assert_ne!(work, dir.0);
}

#[test]
fn chats_are_kept_per_place() {
    let dir = temp_dir("places");
    let mut a = MeSession::default();
    a.project = "alpha".to_string();
    push_message(&mut a, "user", "在 alpha".to_string(), Vec::new());
    save_session(&dir, &a).unwrap();

    switch_place(&mut a, &dir, "project", "beta", true).unwrap();
    assert!(a.messages.is_empty());
    push_message(&mut a, "user", "在 beta".to_string(), Vec::new());
    save_session(&dir, &a).unwrap();

    switch_place(&mut a, &dir, "project", "alpha", true).unwrap();
    assert_eq!(a.messages.len(), 1);
    assert_eq!(a.messages[0].content, "在 alpha");

    switch_place(&mut a, &dir, "config", "alpha", true).unwrap();
    assert!(a.messages.is_empty());
    assert_eq!(a.mode, "config");
}

#[tokio::test]
async fn cancel_stops_the_loop_between_steps() {
    let dir = temp_dir("cancel");
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_ask = calls.clone();
    let cancel_ask = cancel.clone();
    let ask = move |_messages: Vec<Value>| {
        let n = calls_ask.fetch_add(1, Ordering::SeqCst);
        let cancel_ask = cancel_ask.clone();
        async move {
            if n == 0 {
                Ok("```bash\necho one\n```".to_string())
            } else {
                cancel_ask.store(true, Ordering::SeqCst);
                Ok("```bash\necho two\n```".to_string())
            }
        }
    };
    let result = run_loop(
        ask,
        |_| {},
        true,
        &dir,
        &[],
        None,
        "",
        "system",
        LoopLimits::default(),
        false,
        &cancel,
    )
    .await
    .unwrap();
    assert!(result.content.contains("停下"));
    assert_eq!(result.steps.len(), 1);
    assert!(result.pending.is_none());
}

struct TmpDir(PathBuf);

impl std::ops::Deref for TmpDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(name: &str) -> TmpDir {
    let dir = std::env::temp_dir().join(format!(
        "sugt-me-{}-{}-{}",
        name,
        std::process::id(),
        now_ms()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    TmpDir(dir)
}
