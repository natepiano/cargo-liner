//! Status and cut text are checked in rendered cells.

use std::sync::Arc;

use super::*;
use crate::census::Activity;
use crate::census::ShellTimerObservation;
use crate::census::activity::RunLocation;
use crate::census::activity::VerifyStep;
use crate::census::transcript::SubagentRole;
use crate::footer_colors::constants::MINIMUM_CONTRAST;
use crate::footer_colors::tests;
use crate::theme;

/// The acceptance rule fixes stem coverage at 208/255 independently of production.
fn busy_stem_contrast(ink: Color, ground: Color) -> f64 {
    #[expect(
        clippy::panic,
        reason = "a theme fixture must have concrete color channels"
    )]
    fn channels(color: Color) -> [u8; 3] {
        // Only conversion to xterm channels is shared; coverage and ratio are measured here.
        match tui_pane::blend_color(color, color, 0) {
            Color::Rgb(red, green, blue) => [red, green, blue],
            other => panic!("a concrete theme color must have RGB channels: {other:?}"),
        }
    }
    let ink = channels(ink);
    let ground_channels = channels(ground);
    let mixed: [u8; 3] = std::array::from_fn(|index| {
        u8::try_from((u32::from(ink[index]) * 208 + u32::from(ground_channels[index]) * 47) / 255)
            .expect("weighted channels fit a byte")
    });
    tests::contrast_ratio(Color::Rgb(mixed[0], mixed[1], mixed[2]), ground)
}

/// Draw a frame with every Busy row kind while retaining the caller's style cache.
fn busy_frame(ground: Color, activity: Activity, colors: &FooterColors) -> Buffer {
    let mut root = agent(10, "owner", HOUR, None);
    root.activity = AgentActivity::Active(activity);
    root.children = [
        ("thinking-thread", ChildKind::Thread, Activity::Thinking),
        ("running-thread", ChildKind::Thread, Activity::Running),
        (
            "thinking-subagent",
            ChildKind::Subagent(SubagentRole::Unrecorded),
            Activity::Thinking,
        ),
        (
            "running-subagent",
            ChildKind::Subagent(SubagentRole::Unrecorded),
            Activity::Running,
        ),
    ]
    .into_iter()
    .map(|(label, kind, activity)| ChildRow {
        activity: AgentActivity::Active(activity),
        ..child(0, kind, None, label, MINUTE)
    })
    .collect();
    root.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "running-shell",
        MINUTE,
    ));
    let member = AgentRow {
        activity: AgentActivity::Active(activity),
        ..agent(30, "managed-session", MINUTE, None)
    };
    let mut entry = entry(&root, None);
    entry.lines.push(TreeLine {
        dispatch_label: DispatchLabel::Original,
        machine:        "natedev",
        location:       SessionLocation::SameMachine,
        row:            &member,
        hue:            RainbowHue::Red,
        rails:          Vec::new(),
        branch:         Branch::Last,
        subject:        LineSubject::Agent(RoleLabel::Unlabeled),
    });
    let rect = area(180, 15);
    let mut buffer = Buffer::empty(rect);
    buffer.set_style(rect, Style::default().bg(ground));
    draw(
        &mut buffer,
        rect,
        &entry,
        std::slice::from_ref(&entry),
        NOW,
        AccountColumn::Hidden,
        colors,
    );
    buffer
}

#[test]
fn busy_status_words_share_one_drawn_ink_across_row_kinds_and_frame_changes() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let colors = FooterColors::default();
    let mut failures = Vec::new();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            for focused in [false, true] {
                let ground = tui_pane::pane_background(focused);
                // Keep the style cache alive while a session changes state between frames.
                let mut frame_inks = Vec::new();
                for activity in [Activity::Thinking, Activity::Running, Activity::Thinking] {
                    let buffer = busy_frame(ground, activity, &colors);
                    let status_x = find(&buffer, "status").expect("status header").0;
                    let mut inks = Vec::new();
                    for (label, word) in [
                        ("thinking-thread", "thinking"),
                        ("running-thread", "running"),
                        ("thinking-subagent", "thinking"),
                        ("running-subagent", "running"),
                        ("running-shell", "running"),
                        ("managed-session", AgentActivity::Active(activity).text()),
                    ] {
                        let y = find(&buffer, label).expect("activity row").1;
                        let actual: String = (status_x..status_x + summary::cell_width(word))
                            .map(|x| buffer[(x, y)].symbol())
                            .collect();
                        assert_eq!(actual, word, "{label}");
                        let cell = &buffer[(status_x, y)];
                        let ratio = busy_stem_contrast(cell.fg, cell.bg);
                        if ratio < 4.5 {
                            failures.push(format!("{name} tint={tint} focused={focused} {label}: {cell:?} covered={ratio:.6}"));
                        }
                        inks.push(cell.fg);
                    }
                    let (x, y) = find(&buffer, AgentActivity::Active(activity).text())
                        .expect("owner header status");
                    inks.push(buffer[(x, y)].fg);
                    if inks.iter().any(|ink| *ink != inks[0]) {
                        failures.push(format!(
                            "{name} tint={tint} focused={focused} {activity:?}: {inks:?}"
                        ));
                    }
                    frame_inks.push(inks[0]);
                }
                if frame_inks.iter().any(|ink| *ink != frame_inks[0]) {
                    failures.push(format!(
                        "{name} tint={tint} focused={focused} frame changes: {frame_inks:?}"
                    ));
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(failures.is_empty(), "Busy ink failures: {failures:#?}");
}

fn check_statuses(ground: Color) {
    let mut root = agent(1, "fixture", 60, None);
    let activities = [
        AgentActivity::Idle,
        AgentActivity::Active(Activity::Waiting),
        AgentActivity::Active(Activity::Thinking),
        AgentActivity::Active(Activity::Reading),
        AgentActivity::Active(Activity::Verify {
            step:     VerifyStep::Check,
            location: RunLocation::Local,
        }),
    ];
    root.children = activities
        .iter()
        .enumerate()
        .map(|(index, &activity)| {
            let mut row = child(
                0,
                ChildKind::Subagent(SubagentRole::Unrecorded),
                None,
                &format!("job-{index}"),
                10,
            );
            row.activity = activity;
            row
        })
        .collect();
    let cell = entry(&root, None);
    let rect = area(150, 15);
    let mut buffer = Buffer::empty(rect);
    buffer.set_style(rect, Style::default().bg(ground));
    draw(
        &mut buffer,
        rect,
        &cell,
        std::slice::from_ref(&cell),
        NOW,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    for activity in activities {
        let word = activity.text();
        let (x, y) = find(&buffer, word).expect("drawn activity word");
        for offset in 0..summary::cell_width(word) {
            let cell = &buffer[(x + offset, y)];
            let ratio = tests::contrast_ratio(cell.fg, cell.bg);
            assert!(ratio >= MINIMUM_CONTRAST, "{word}: {cell:?} is {ratio}:1");
        }
    }
}

#[test]
fn status_words_meet_contrast_on_focused_and_unfocused_tiles_in_every_theme() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    tui_pane::set_focused_pane_tint(true);
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for transparency in [false, true] {
            tui_pane::set_transparent_background(transparency);
            for focused in [false, true] {
                check_statuses(tui_pane::pane_background(focused));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
}

#[test]
fn drawn_cuts_remove_whitespace_before_the_truncation_mark() {
    for text in [
        "scripts into a long directory",
        "You are doing a longer job",
        "scripts into\t a longer directory",
    ] {
        let width = if text.starts_with("You") { 5 } else { 14 };
        let line = cut_line(
            vec![Span::styled(text, Style::default().fg(Color::Cyan))],
            width,
        );
        let rect = area(width, 1);
        let mut buffer = Buffer::empty(rect);
        Paragraph::new(line).render(rect, &mut buffer);
        let drawn: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(drawn.contains(TRUNCATION_MARK), "{drawn:?}");
        let before = drawn.split(TRUNCATION_MARK).next().expect("cut prefix");
        assert!(!before.ends_with(char::is_whitespace), "{drawn:?}");
        assert_eq!(buffer[(summary::cell_width(before), 0)].fg, Color::Cyan);
        assert!(summary::truncated(text, usize::from(width)).ends_with(TRUNCATION_MARK));
    }
    assert_eq!(summary::truncated("x", 0), "");
    assert_eq!(summary::truncated("long", 1), "…");
    assert_eq!(summary::truncated("fits ", 5), "fits ");
}

fn check_role_and_account_cells(ground: Color) {
    let colors = FooterColors::parse(
        "role.showrunner=bright-magenta\nrole.unit-director=bright-blue\nrole.worker=bright-cyan\nrole.unassigned=purple\naccount.claude1=brown\naccount.claude2=magenta\naccount.claude3=blue\naccount.claude4=orange\naccount.codex1=pink\naccount.codex2=teal",
    );
    let root = agent(10, "root", 60, None);
    let labels = [
        "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
    ];
    let roles = [
        (RoleLabel::Stated(HierarchyRole::Showrunner), "showrunner"),
        (RoleLabel::Stated(HierarchyRole::Unit), "unit"),
        (RoleLabel::Stated(HierarchyRole::Impl), "impl"),
        (RoleLabel::Unlabeled, "session"),
    ];
    let members: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| AgentRow {
            account: serde_json::from_value(
                serde_json::json!({"home": format!("~/.{label}"), "label":label}),
            )
            .expect("account fixture"),
            ..agent(
                u32::try_from(index + 20).expect("fixture pid"),
                &format!("member-{index}"),
                20,
                None,
            )
        })
        .collect();
    let mut cell = entry(&root, None);
    for (index, member) in members.iter().enumerate() {
        cell.lines.push(TreeLine {
            dispatch_label: DispatchLabel::Original,
            machine:        "natedev",
            location:       SessionLocation::SameMachine,
            row:            member,
            hue:            RainbowHue::Red,
            rails:          Vec::new(),
            branch:         Branch::Last,
            subject:        LineSubject::Agent(roles[index % roles.len()].0),
        });
    }
    let rect = area(180, 20);
    let mut buffer = Buffer::empty(rect);
    buffer.set_style(rect, Style::default().bg(ground));
    draw(
        &mut buffer,
        rect,
        &cell,
        std::slice::from_ref(&cell),
        NOW,
        AccountColumn::Shown,
        &colors,
    );
    let role_x = find(&buffer, "role").expect("role header").0;
    let account_x = find(&buffer, "account").expect("account header").0;
    for (index, member) in members.iter().enumerate() {
        let y = find(&buffer, &member.name).expect("member row").1;
        for (x, word) in [
            (role_x, roles[index % roles.len()].1),
            (account_x, labels[index]),
        ] {
            for offset in 0..summary::cell_width(word) {
                let cell = &buffer[(x + offset, y)];
                assert!(
                    tests::contrast_ratio(cell.fg, cell.bg) >= MINIMUM_CONTRAST,
                    "{word}: {cell:?}"
                );
            }
        }
    }
}

#[test]
fn tree_role_and_account_words_meet_contrast_on_every_tile_ground() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    tui_pane::set_focused_pane_tint(true);
    for variant in theme::builtins() {
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for transparency in [false, true] {
            tui_pane::set_transparent_background(transparency);
            for focused in [false, true] {
                check_role_and_account_cells(tui_pane::pane_background(focused));
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
}

#[test]
fn status_cells_keep_glyph_coverage_contrast_on_every_drawn_pane_ground() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut failures = Vec::new();
    let mut root = agent(10, "status-owner", HOUR, None);
    root.children = [
        AgentActivity::Idle,
        AgentActivity::Active(Activity::Waiting),
        AgentActivity::Active(Activity::Thinking),
        AgentActivity::Active(Activity::Verify {
            step:     VerifyStep::Check,
            location: RunLocation::Local,
        }),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, activity)| ChildRow {
        activity,
        ..child(
            0,
            ChildKind::Subagent(SubagentRole::Unrecorded),
            None,
            &format!("activity-{index}"),
            MINUTE,
        )
    })
    .collect();
    root.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "compile-project",
        MINUTE,
    ));
    root.children.push(ChildRow {
        kind: ChildKind::Shell(ShellTimerObservation::ObservedCountdown(Timer {
            started:  NOW - 92,
            deadline: NOW + 228,
        })),
        ..child(
            0,
            ChildKind::Shell(ShellTimerObservation::Untimed),
            Some(22),
            "progress-clock",
            92,
        )
    });
    let entry = entry(&root, None);
    let rect = area(180, 14);
    // Reuse the cache across theme, focus and tint changes, as the app does.
    let colors = FooterColors::default();
    tui_pane::set_transparent_background(false);
    for variant in theme::builtins() {
        let name = variant.id.as_str().to_owned();
        tui_pane::set_active_theme(Arc::new(variant.theme));
        for tint in [false, true] {
            tui_pane::set_focused_pane_tint(tint);
            for focused in [false, true] {
                let ground = tui_pane::pane_background(focused);
                let mut buffer = Buffer::empty(rect);
                buffer.set_style(rect, Style::default().bg(ground));
                draw(
                    &mut buffer,
                    rect,
                    &entry,
                    std::slice::from_ref(&entry),
                    NOW,
                    AccountColumn::Hidden,
                    &colors,
                );
                let status_x = find(&buffer, TREE_STATUS_HEADER).expect("status header").0;
                for row in &root.children {
                    let y = find(&buffer, &row.name).expect("status row").1;
                    let cell = &buffer[(status_x, y)];
                    let ratio = tests::covered_role_contrast(cell.fg, cell.bg);
                    eprintln!(
                        "{name} tint={tint} focused={focused} {}: fg={:?} bg={:?} covered={ratio:.6}",
                        row.name, cell.fg, cell.bg
                    );
                    if ratio < MINIMUM_CONTRAST {
                        failures.push(format!(
                            "{name} tint={tint} focused={focused} {}: {cell:?} covered={ratio:.6}",
                            row.name
                        ));
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "drawn status coverage failures: {failures:#?}"
    );
}

#[test]
fn light_running_and_waiting_stems_keep_readable_contrast() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    tui_pane::set_transparent_background(false);
    tui_pane::set_focused_pane_tint(false);
    tui_pane::set_active_theme(Arc::new(
        theme::builtins()
            .into_iter()
            .find(|variant| variant.id.as_str() == crate::constants::DEFAULT_LIGHT_THEME)
            .expect("built-in light theme")
            .theme,
    ));
    let mut root = agent(10, "status-owner", HOUR, None);
    root.children = [Activity::Running, Activity::Waiting]
        .into_iter()
        .enumerate()
        .map(|(index, activity)| ChildRow {
            activity: AgentActivity::Active(activity),
            ..child(
                0,
                ChildKind::Subagent(SubagentRole::Unrecorded),
                None,
                &format!("stem-{index}"),
                MINUTE,
            )
        })
        .collect();
    let entry = entry(&root, None);
    let rect = area(180, 10);
    let mut buffer = Buffer::empty(rect);
    buffer.set_style(rect, Style::default().bg(tui_pane::pane_background(false)));
    draw(
        &mut buffer,
        rect,
        &entry,
        std::slice::from_ref(&entry),
        NOW,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    for word in ["running", "waiting"] {
        let (x, y) = find(&buffer, word).expect("drawn status word");
        let cell = &buffer[(x, y)];
        let ratio = tests::covered_role_contrast(cell.fg, cell.bg);
        eprintln!(
            "{word}: fg={:?} bg={:?} stem contrast={ratio:.6}",
            cell.fg, cell.bg
        );
        assert!(
            ratio >= MINIMUM_CONTRAST,
            "{word}: {cell:?} stem contrast={ratio:.6}"
        );
    }
}
