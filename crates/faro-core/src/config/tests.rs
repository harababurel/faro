use super::*;

/// The template with every `# key = value` line uncommented (explanations stay comments).
fn uncommented(template: &str) -> String {
    template
        .lines()
        .map(|line| match line.strip_prefix("# ") {
            Some(rest) if rest.contains(" = ") && !rest.ends_with('.') => rest.to_owned(),
            Some(_) | None => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_fresh(text: &str) -> Parsed {
    parse(text, &Config::default())
}

#[test]
fn the_template_parses_to_the_defaults() {
    let parsed = parse_fresh(&template(""));
    assert_eq!(parsed.problems, vec![]);
    assert_eq!(parsed.config, Config::default());
}

#[test]
fn every_setting_is_in_the_template_with_its_default() {
    let template = template("");
    let defaults = Config::default();
    for setting in SETTINGS {
        let (_, key) = setting.split();
        let line = format!("# {key} = {}\n", setting.get(&defaults));
        assert!(template.contains(&line), "{} missing: {line}", setting.path);
        assert!(!setting.help.is_empty() && !setting.help.contains('\n'));
    }
}

#[test]
fn uncommenting_every_default_changes_nothing() {
    let text = uncommented(&template(""));
    assert!(text.contains("\nlayout = \"bubbles\""), "{text}");
    let parsed = parse_fresh(&text);
    assert_eq!(parsed.problems, vec![]);
    assert_eq!(parsed.config, Config::default());
}

#[test]
fn key_constants_name_settings() {
    for path in [
        key::THEME,
        key::EMOJI,
        key::ALLOWLIST,
        key::LAYOUT,
        key::BUBBLE_STYLE,
        key::DENSITY,
        key::TIMESTAMPS,
        key::AUTO_DOWNLOAD,
        key::INLINE_IMAGES,
        key::VOICE_PLAYER,
        key::VOICE_TRANSCRIBE,
        key::VOICE_TRANSCRIBE_COMMAND,
        key::VOICE_TRANSCRIBE_MODEL,
        key::VOICE_TRANSCRIBE_URL,
        key::VOICE_TRANSCRIBE_LANGUAGE,
        key::VOICE_TRANSCRIBE_MAX_SECS,
        key::MOUSE,
    ] {
        assert!(setting(path).is_some(), "{path}");
    }
}

#[test]
fn settings_are_read() {
    let parsed = parse_fresh(
        "theme = \"other\"\n\
         [ui]\nemoji = \"compatible\"\n\
         [conversations]\nallowlist = true\n\
         [messages]\nlayout = \"irc\"\ndensity = \"compact\"\ntimestamps = \"relative\"\n\
         [media]\nauto_download = \"all\"\ninline_images = \"halfblocks\"\n\
         [voice]\nplayer = \"mpv\"\ntranscribe = \"local\"\ntranscribe_command = \"whisper-cli\"\n\
         transcribe_model = \"small\"\ntranscribe_language = \"en\"\ntranscribe_max_secs = 180\n\
         [notifications]\nbell = false\ndesktop = \"osc\"\nprivacy = \"preview\"\n\
         [mouse]\nenabled = false\n",
    );
    assert_eq!(parsed.problems, vec![]);
    let c = parsed.config;
    assert_eq!(c.theme, "other");
    assert_eq!(c.ui.emoji, EmojiChoice::Compatible);
    assert!(c.conversations.allowlist);
    assert_eq!(c.messages.layout, MessageLayout::Irc);
    assert_eq!(c.messages.density, MessageDensity::Compact);
    assert_eq!(c.messages.timestamps, TimestampFormat::Relative);
    assert_eq!(c.media.auto_download, AutoDownload::All);
    assert_eq!(c.media.inline_images, InlineImages::Halfblocks);
    assert_eq!(c.voice.player, "mpv");
    assert_eq!(c.voice.transcribe, TranscribeMode::Local);
    assert_eq!(c.voice.transcribe_command, "whisper-cli");
    assert_eq!(c.voice.transcribe_model, "small");
    assert_eq!(c.voice.transcribe_language, "en");
    assert_eq!(c.voice.transcribe_max_secs, 180);
    assert!(!c.notifications.bell);
    assert_eq!(c.notifications.desktop, DesktopNotifications::Osc);
    assert_eq!(c.notifications.privacy, NotificationPrivacy::Preview);
    assert!(!c.mouse.enabled);
}

#[test]
fn inline_and_dotted_tables_work_too() {
    let parsed = parse_fresh("messages = { layout = \"irc\" }\nmouse.enabled = false\n");
    assert_eq!(parsed.problems, vec![]);
    assert_eq!(parsed.config.messages.layout, MessageLayout::Irc);
    assert!(!parsed.config.mouse.enabled);
}

#[test]
fn a_bad_value_keeps_the_previous_one_and_names_the_key() {
    let mut previous = Config::default();
    previous.messages.layout = MessageLayout::Irc;
    let parsed = parse(
        "[messages]\nlayout = \"bubble\"\n[mouse]\nenabled = \"no\"\n",
        &previous,
    );
    assert_eq!(parsed.config.messages.layout, MessageLayout::Irc);
    assert!(parsed.config.mouse.enabled);
    assert_eq!(
        parsed.problems,
        vec![
            Problem::error(
                "messages.layout: expected \"bubbles\" or \"irc\", not \"bubble\"; keeping \"irc\""
            ),
            Problem::error("mouse.enabled: expected true or false, not \"no\"; keeping true"),
        ]
    );
}

#[test]
fn a_missing_setting_goes_back_to_its_default() {
    let mut previous = Config::default();
    previous.messages.layout = MessageLayout::Irc;
    let parsed = parse("", &previous);
    assert_eq!(parsed.config, Config::default());
}

#[test]
fn the_old_bubble_corners_setting_is_still_read_with_a_warning() {
    let parsed = parse_fresh("[messages]\nbubble_corners = \"square\"\n");
    assert_eq!(parsed.config.messages.bubble_style, BubbleStyle::Bar);
    assert_eq!(
        parsed.problems,
        vec![Problem::warning(
            "messages.bubble_corners is now messages.bubble_style: write bubble_style = \"bar\""
        )]
    );
    let parsed = parse_fresh("[messages]\nbubble_corners = \"rounded\"\n");
    assert_eq!(parsed.config.messages.bubble_style, BubbleStyle::Rounded);
    // The new name wins.
    let parsed =
        parse_fresh("[messages]\nbubble_corners = \"square\"\nbubble_style = \"rounded\"\n");
    assert_eq!(parsed.config.messages.bubble_style, BubbleStyle::Rounded);
    assert_eq!(
        parsed.problems,
        vec![Problem::warning(
            "messages.bubble_corners is replaced by messages.bubble_style; ignored (remove it)"
        )]
    );
    let parsed = parse_fresh("[messages]\nbubble_corners = \"oval\"\n");
    assert_eq!(parsed.config.messages.bubble_style, BubbleStyle::Auto);
    assert_eq!(parsed.problems.len(), 1);
    assert_eq!(parsed.problems[0].severity, Severity::Error);
}

#[test]
fn a_broken_file_keeps_the_whole_previous_config() {
    let mut previous = Config::default();
    previous.mouse.enabled = false;
    let parsed = parse("[messages]\nlayout = \"irc\"\nmouse = \n", &previous);
    assert_eq!(parsed.config, previous);
    assert_eq!(parsed.problems.len(), 1);
    let problem = &parsed.problems[0];
    assert_eq!(problem.severity, Severity::Error);
    assert!(problem.text.starts_with("line 3: "), "{}", problem.text);
}

#[test]
fn unknown_and_planned_keys_are_warnings() {
    let parsed = parse_fresh(
        "colour = 1\n[messages]\nlayout = \"irc\"\ndensity = \"compact\"\n\
         [voice]\ntranscribe_api_key = \"sk-secret\"\n[nope]\nx = 1\n",
    );
    assert_eq!(parsed.config.messages.layout, MessageLayout::Irc);
    assert_eq!(parsed.config.messages.density, MessageDensity::Compact);
    assert_eq!(
        parsed.problems,
        vec![
            Problem::warning("unknown setting colour; ignored"),
            Problem::warning(
                "voice.transcribe_api_key is ignored: API keys are never read from config.toml; \
                 set $FARO_TRANSCRIBE_API_KEY or $OPENAI_API_KEY"
            ),
            Problem::warning("unknown setting nope; ignored"),
        ]
    );
}

#[test]
fn keybindings_are_collected_in_order() {
    let parsed = parse_fresh(
        "[keybindings.global]\n\"ctrl+k\" = \"palette.open\"\n\"alt+j\" = \"none\"\n\
         [keybindings.messages]\nx = 3\n",
    );
    assert_eq!(
        parsed.config.keybindings,
        vec![
            Keybinding {
                context: "global".into(),
                key: "ctrl+k".into(),
                action: "palette.open".into(),
            },
            Keybinding {
                context: "global".into(),
                key: "alt+j".into(),
                action: "none".into(),
            },
        ]
    );
    assert_eq!(
        parsed.problems,
        vec![Problem::error(
            "keybindings.messages: \"x\" should name an action in quotes"
        )]
    );
}

fn layout(name: &str) -> ConfigValue {
    ConfigValue::Text(name.into())
}

#[test]
fn setting_a_commented_default_uncomments_its_line() {
    let original = template("[keybindings.global]\n# \"ctrl+p\" = \"palette.open\"\n");
    let edited = set(&original, key::LAYOUT, &layout("irc")).unwrap();
    let changed: Vec<(&str, &str)> = original
        .lines()
        .zip(edited.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        changed,
        vec![("# layout = \"bubbles\"", "layout = \"irc\"")]
    );
    assert_eq!(original.lines().count(), edited.lines().count());
    assert_eq!(
        parse_fresh(&edited).config.messages.layout,
        MessageLayout::Irc
    );

    // Again: now the key is set, so only its value changes.
    let back = set(&edited, key::LAYOUT, &layout("bubbles")).unwrap();
    assert_eq!(back, original.replace("# layout = ", "layout = "));
}

#[test]
fn setting_keeps_comments_next_to_the_value() {
    let text = "# top\n[messages]\n  layout   = \"irc\"   # mine\n# end\n";
    let edited = set(text, key::LAYOUT, &layout("bubbles")).unwrap();
    assert_eq!(
        edited,
        "# top\n[messages]\n  layout   = \"bubbles\"   # mine\n# end\n"
    );
    assert_eq!(
        set(&edited, key::LAYOUT, &layout("bubbles")).unwrap(),
        edited
    );
}

#[test]
fn a_commented_line_in_another_table_is_left_alone() {
    let text = "[mouse]\n# layout = \"bubbles\"\n[messages]\n# other comment\n";
    let edited = set(text, key::LAYOUT, &layout("irc")).unwrap();
    assert!(edited.starts_with("[mouse]\n# layout = \"bubbles\"\n[messages]\n"));
    assert_eq!(
        parse_fresh(&edited).config.messages.layout,
        MessageLayout::Irc
    );
}

#[test]
fn setting_adds_missing_keys_and_tables() {
    let edited = set("# my config\n", key::LAYOUT, &layout("irc")).unwrap();
    assert_eq!(edited, "# my config\n\n[messages]\nlayout = \"irc\"\n");
    let edited = set(&edited, key::MOUSE, &ConfigValue::Bool(false)).unwrap();
    let edited = set(&edited, key::THEME, &layout("faro-dark")).unwrap();
    let parsed = parse_fresh(&edited);
    assert_eq!(parsed.problems, vec![]);
    assert_eq!(parsed.config.messages.layout, MessageLayout::Irc);
    assert!(!parsed.config.mouse.enabled);
    assert_eq!(
        edited,
        "# my config\n\ntheme = \"faro-dark\"\n\n[messages]\nlayout = \"irc\"\n\n\
         [mouse]\nenabled = false\n"
    );
}

#[test]
fn a_broken_file_is_not_edited() {
    assert!(matches!(
        set("[messages\n", key::LAYOUT, &layout("irc")),
        Err(EditError::Syntax(_))
    ));
    assert!(matches!(
        set("messages = 3\n", key::LAYOUT, &layout("irc")),
        Err(EditError::NotATable(_))
    ));
}

#[test]
fn crlf_files_keep_their_line_endings() {
    let text = "[messages]\r\n# layout = \"bubbles\"\r\n";
    let edited = set(text, key::LAYOUT, &layout("irc")).unwrap();
    assert_eq!(edited, "[messages]\r\nlayout = \"irc\"\r\n");
}

#[test]
fn quoting() {
    assert_eq!(quoted("ctrl+p"), "\"ctrl+p\"");
    assert_eq!(
        parse_fresh(&format!("[keybindings.global]\n{} = \"x\"\n", quoted("\"")))
            .config
            .keybindings[0]
            .key,
        "\""
    );
}

#[test]
fn mentions_finds_set_keys_only() {
    let text = "theme = \"x\"\n[messages]\n# layout = \"irc\"\n[mouse]\nenabled = 3\n";
    assert!(mentions(text, key::THEME));
    assert!(!mentions(text, key::LAYOUT));
    assert!(mentions(text, key::MOUSE));
    assert!(!mentions("[broken", key::MOUSE));
}
