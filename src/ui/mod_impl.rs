use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{
    state::{AppMode, AppState},
    tree::NodeKind,
};

/// Render the entire TUI frame.
pub fn render(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();

    // Split vertically: tree pane | status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let tree_area = chunks[0];
    let status_area = chunks[1];

    // Update the pane height so AppState can adjust scrolling.
    state.pane_height = tree_area.height.saturating_sub(2) as usize; // subtract borders

    render_tree(frame, tree_area, state);
    render_status(frame, status_area, state);

    if state.mode == AppMode::Help {
        render_help_overlay(frame, area);
    }
}

/// Render the hierarchical code tree panel.
fn render_tree(frame: &mut Frame, area: Rect, state: &AppState) {
    let title = if let Some(ref p) = state.current_file {
        format!(" {} ", p.display())
    } else {
        " Terraform — Hierarchical Code Viewer ".to_string()
    };

    let filter_suffix = if !state.filter.is_empty() {
        format!(" [filter: {}]", state.filter)
    } else if state.mode == AppMode::Filter {
        " [filter: _]".to_string()
    } else {
        String::new()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("{title}{filter_suffix}"))
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.visible_ids.is_empty() {
        let hint = Paragraph::new("No file loaded. Pass a source file path as a command-line argument.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, inner);
        return;
    }

    // Build list items from visible nodes.
    let items: Vec<ListItem> = state
        .visible_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let node = state.tree.get(id).expect("visible id must exist");
            let is_selected = i == state.cursor;

            // Indentation.
            let indent = "  ".repeat(node.depth);

            // Collapse/expand indicator.
            let expand_icon = if node.is_leaf() {
                "  "
            } else if node.collapsed {
                "▶ "
            } else {
                "▼ "
            };

            // Kind icon.
            let kind_icon = match node.kind {
                NodeKind::Module => "mod",
                NodeKind::File => "fil",
                NodeKind::Class => "cls",
                NodeKind::Function => "fn ",
                NodeKind::Block => "{ }",
                NodeKind::Line => "   ",
            };

            let name_style = if is_selected {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                kind_color_style(&node.kind)
            };

            let detail_text = node
                .detail
                .as_deref()
                .map(|d| format!("  {d}"))
                .unwrap_or_default();

            let line = Line::from(vec![
                Span::raw(format!("{indent}{expand_icon}")),
                Span::styled(
                    format!("[{kind_icon}] {}", node.name),
                    name_style,
                ),
                Span::styled(detail_text, Style::default().fg(Color::DarkGray)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.cursor.saturating_sub(state.scroll_offset)));

    // Slice the items according to scroll offset.
    let end = (state.scroll_offset + inner.height as usize).min(items.len());
    let visible_items: Vec<ListItem> = items
        .into_iter()
        .skip(state.scroll_offset)
        .take(end.saturating_sub(state.scroll_offset))
        .collect();

    let list = List::new(visible_items).highlight_style(
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, inner, &mut list_state);
}

/// Render the one-line status bar at the bottom.
fn render_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let mode_label = match state.mode {
        AppMode::Normal => Span::styled(
            " NORMAL ",
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        AppMode::Filter => Span::styled(
            " FILTER ",
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        AppMode::Help => Span::styled(
            "  HELP  ",
            Style::default()
                .bg(Color::Magenta)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    };

    let status_text = Span::raw(format!(" {} ", state.status));

    let right_hint = Span::styled(
        " ? Help  q Quit ",
        Style::default().fg(Color::DarkGray),
    );

    let line = Line::from(vec![mode_label, status_text, right_hint]);
    let para = Paragraph::new(line).style(Style::default().bg(Color::Reset));
    frame.render_widget(para, area);
}

/// Render the help overlay as a centered popup.
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(60, 70, area);

    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(vec![Span::styled(
            "  Keyboard Shortcuts",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Navigation",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  Up / k       Move cursor up"),
        Line::from("  Down / j     Move cursor down"),
        Line::from("  PgUp         Page up"),
        Line::from("  PgDn         Page down"),
        Line::from("  g / Home     Jump to top"),
        Line::from("  G / End      Jump to bottom"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Collapse / Expand",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  Space / Enter  Toggle collapse/expand"),
        Line::from("  [            Collapse all"),
        Line::from("  ]            Expand all"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Filter",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  /            Enter filter mode"),
        Line::from("  Enter        Confirm filter"),
        Line::from("  Esc          Clear filter / cancel"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  General",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  ? / F1       Toggle this help"),
        Line::from("  q / Ctrl+C   Quit"),
    ];

    let help_block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let para = Paragraph::new(help_text)
        .block(help_block)
        .wrap(Wrap { trim: false });

    frame.render_widget(para, popup_area);
}

/// Return the style for a given `NodeKind`.
fn kind_color_style(kind: &NodeKind) -> Style {
    match kind {
        NodeKind::Module => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        NodeKind::File => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        NodeKind::Class => Style::default().fg(Color::Yellow),
        NodeKind::Function => Style::default().fg(Color::LightBlue),
        NodeKind::Block => Style::default().fg(Color::Gray),
        NodeKind::Line => Style::default().fg(Color::Reset),
    }
}

/// Create a centered rectangle of `percent_x` x `percent_y` of the given area.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
