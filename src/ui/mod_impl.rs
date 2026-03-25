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
use crate::graph::entity::EntityKind;
use crate::graph::tree::NodeKind as GraphNodeKind;

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
    state.pane_height = tree_area.height.saturating_sub(2) as usize;

    // Use the graph tree renderer when a Navigator is available.
    if state.navigator.is_some() {
        render_graph_tree(frame, tree_area, state);
    } else {
        render_tree(frame, tree_area, state);
    }
    render_status(frame, status_area, state);

    if state.mode == AppMode::Help {
        render_help_overlay(frame, area);
    }
}

// ── Graph tree renderer ───────────────────────────────────────────────────────

/// Render the reference-based graph tree produced by the [`Navigator`].
fn render_graph_tree(frame: &mut Frame, area: Rect, state: &AppState) {
    let title = if let Some(ref p) = state.current_path {
        format!(" {} [graph] ", p.display())
    } else {
        " Terraform — Graph View ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.graph_visible.is_empty() {
        let hint = Paragraph::new(
            "No entities found. Use l/Right to zoom in, h/Left to zoom out.",
        )
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, inner);
        return;
    }

    let nav = match &state.navigator {
        Some(n) => n,
        None => return,
    };

    // Determine which nodes are reference-connected to the cursor node.
    let cursor_entity = state
        .graph_visible
        .get(state.graph_cursor)
        .and_then(|&(nid, _)| nav.tree().get(nid))
        .map(|n| n.entity_id);

    let ref_connected: std::collections::HashSet<crate::graph::entity::EntityId> =
        if let Some(cursor_eid) = cursor_entity {
            // Collect tree nodes whose entity_id appears as a direct parent or
            // child of the cursor's tree node in the view tree.
            let cursor_tree_node = state
                .graph_visible
                .get(state.graph_cursor)
                .map(|&(nid, _)| nid);
            let mut connected = std::collections::HashSet::new();
            if let Some(cnid) = cursor_tree_node {
                if let Some(cnode) = nav.tree().get(cnid) {
                    // Children of cursor node
                    for &child_nid in &cnode.children {
                        if let Some(child) = nav.tree().get(child_nid) {
                            connected.insert(child.entity_id);
                        }
                    }
                    // Parent of cursor node
                    if let Some(parent_nid) = cnode.parent {
                        if let Some(parent) = nav.tree().get(parent_nid) {
                            connected.insert(parent.entity_id);
                        }
                    }
                }
            }
            connected.remove(&cursor_eid);
            connected
        } else {
            std::collections::HashSet::new()
        };

    let items: Vec<ListItem> = state
        .graph_visible
        .iter()
        .enumerate()
        .map(|(i, &(node_id, depth))| {
            build_graph_tree_item(
                state,
                node_id,
                depth,
                i == state.graph_cursor,
                &ref_connected,
            )
        })
        .collect();

    let mut list_state = ListState::default();
    let selection = state.graph_cursor.saturating_sub(state.graph_scroll_offset);

    let end = (state.graph_scroll_offset + inner.height as usize).min(items.len());
    let visible_items: Vec<ListItem> = items
        .into_iter()
        .skip(state.graph_scroll_offset)
        .take(end.saturating_sub(state.graph_scroll_offset))
        .collect();

    // Clamp selection to the visible window so StatefulList never receives an
    // out-of-range index.
    if !visible_items.is_empty() {
        list_state.select(Some(selection.min(visible_items.len() - 1)));
    }

    let list = List::new(visible_items).highlight_style(
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, inner, &mut list_state);
}

fn build_graph_tree_item<'a>(
    state: &'a AppState,
    node_id: crate::graph::tree::GraphTreeNodeId,
    depth: usize,
    is_selected: bool,
    ref_connected: &std::collections::HashSet<crate::graph::entity::EntityId>,
) -> ListItem<'a> {
    let nav = match &state.navigator {
        Some(n) => n,
        None => return ListItem::new("?"),
    };

    let graph_node = match nav.tree().get(node_id) {
        Some(n) => n,
        None => return ListItem::new("?"),
    };

    let entity = match nav.entity(graph_node.entity_id) {
        Some(e) => e,
        None => return ListItem::new("?"),
    };

    let indent = "  ".repeat(depth);

    // Expand / fold icon.
    let is_folded = state.graph_folded.contains(&graph_node.entity_id);
    let fold_icon = if !graph_node.children.is_empty() {
        if is_folded { "▶ " } else { "▼ " }
    } else {
        "  "
    };

    // Cycle / Ref annotations.
    let cycle_marker = match graph_node.kind {
        GraphNodeKind::Cycle => " ↺",
        GraphNodeKind::Ref => " →",
        GraphNodeKind::Normal => "",
    };

    let kind_icon = entity_kind_icon(&entity.kind);

    let name_style = if is_selected {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if ref_connected.contains(&graph_node.entity_id) {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::UNDERLINED)
    } else {
        entity_kind_style(&entity.kind)
    };

    let line = Line::from(vec![
        Span::raw(format!("{indent}{fold_icon}")),
        Span::styled(format!("[{kind_icon}] {}", entity.name), name_style),
        Span::styled(
            cycle_marker.to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
    ]);

    ListItem::new(line)
}

fn entity_kind_icon(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Folder => "dir",
        EntityKind::Module => "mod",
        EntityKind::File => "fil",
        EntityKind::Class => "cls",
        EntityKind::Function => "fn ",
    }
}

fn entity_kind_style(kind: &EntityKind) -> Style {
    match kind {
        EntityKind::Folder => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        EntityKind::Module => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        EntityKind::File => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        EntityKind::Class => Style::default().fg(Color::Yellow),
        EntityKind::Function => Style::default().fg(Color::LightBlue),
    }
}

// ── Legacy code tree renderer ─────────────────────────────────────────────────

/// Render the hierarchical code tree panel, including the lib section.
fn render_tree(frame: &mut Frame, area: Rect, state: &AppState) {
    let title = if let Some(ref p) = state.current_path {
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
        let hint = Paragraph::new("No file loaded. Pass a source file or directory path as a command-line argument.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, inner);
        return;
    }

    // Build list items: main tree rows.
    // Compute reference-connected nodes for the current cursor.
    let cursor_id = state.visible_ids.get(state.cursor).copied();
    let ref_connected: std::collections::HashSet<usize> = if let Some(cid) = cursor_id {
        state
            .visible_refs
            .iter()
            .filter(|&&(from, to)| from == cid || to == cid)
            .flat_map(|&(from, to)| [from, to])
            .filter(|&x| x != cid)
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let items: Vec<ListItem> = state
        .visible_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let is_ref = ref_connected.contains(&id);
            build_tree_item(state, id, i == state.cursor, state.view_depths.get(i).copied().unwrap_or(0), is_ref)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.cursor.saturating_sub(state.scroll_offset)));

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

fn build_tree_item<'a>(state: &'a AppState, id: usize, is_selected: bool, display_depth: usize, is_ref_connected: bool) -> ListItem<'a> {
    let node = state.tree.get(id).expect("visible id must exist");

    let indent = "  ".repeat(display_depth);

    let expand_icon = if node.kind == NodeKind::SymRef {
        "→ "
    } else if node.is_leaf() {
        "  "
    } else if state.expanded_ids.contains(&id) {
        // Node is expanded+folded: visible as ▶. Space to unfold.
        // (Expanded+unfolded nodes are invisible — their children replaced them.)
        "▶ "
    } else {
        "▷ " // not yet expanded: Right/l to expand granularity
    };

    let kind_icon = kind_icon(&node.kind);

    // For a node with a granularity limit, show it as a small annotation.
    let limit_annotation = if node.kind != NodeKind::SymRef && !node.is_leaf() {
        match &node.granularity_limit {
            Some(limit) => format!(" ~{limit}"),
            None => String::new(),
        }
    } else {
        String::new()
    };

    let name_style = if is_selected {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if is_ref_connected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::UNDERLINED)
    } else {
        kind_color_style(&node.kind)
    };

    let detail_text = node
        .detail
        .as_deref()
        .map(|d| format!("  {d}"))
        .unwrap_or_default();

    let display_name = if node.kind == NodeKind::SymRef {
        node.name.clone()
    } else {
        state.tree.full_path(id)
    };

    let line = Line::from(vec![
        Span::raw(format!("{indent}{expand_icon}")),
        Span::styled(format!("[{kind_icon}] {}", display_name), name_style),
        Span::styled(
            limit_annotation,
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ),
        Span::styled(detail_text, Style::default().fg(Color::DarkGray)),
    ]);

    ListItem::new(line)
}

/// Build a list item showing a single reference edge `from → to`.
#[allow(dead_code)]
fn build_ref_item(state: &AppState, from_id: usize, to_id: usize) -> ListItem<'static> {
    let from_name = state
        .tree
        .get(from_id)
        .map(|n| format!("[{}] {}", kind_icon(&n.kind), n.name))
        .unwrap_or_else(|| format!("#{from_id}"));
    let to_name = state
        .tree
        .get(to_id)
        .map(|n| format!("[{}] {}", kind_icon(&n.kind), n.name))
        .unwrap_or_else(|| format!("#{to_id}"));

    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(from_name, Style::default().fg(Color::LightBlue)),
        Span::styled(
            "  ──→  ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(to_name, Style::default().fg(Color::LightGreen)),
    ]);
    ListItem::new(line)
}

/// Map a [`NodeKind`] to the three-character icon string shown in both the
/// tree rows and the reference-edge rows.
fn kind_icon(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Folder => "dir",
        NodeKind::Module => "mod",
        NodeKind::File => "fil",
        NodeKind::Class => "cls",
        NodeKind::Function => "fn ",
        NodeKind::Block => "{ }",
        NodeKind::Line => "   ",
        NodeKind::SymRef => "ref",
    }
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
    let popup_area = centered_rect(62, 80, area);

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
        Line::from("  Up / k         Move cursor up"),
        Line::from("  Down / j       Move cursor down"),
        Line::from("  PgUp           Page up"),
        Line::from("  PgDn           Page down"),
        Line::from("  g / Home       Jump to top"),
        Line::from("  G / End        Jump to bottom"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Zoom / Fold",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  l / Right      Zoom in (expand entity one level)"),
        Line::from("  h / Left       Zoom out (collapse entity one level)"),
        Line::from("  Space          Fold/unfold reference-tree children"),
        Line::from("  Enter          Same as Space"),
        Line::from("  [  ]           Clear all folds"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  References",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  Nodes connected by references are highlighted (▼ = has"),
        Line::from("  reference children; ↺ = cycle; → = back-edge reference)."),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Filter",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  /              Enter filter mode"),
        Line::from("  Enter          Confirm filter"),
        Line::from("  Esc            Clear filter / cancel"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  General",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from("  ? / F1         Toggle this help"),
        Line::from("  q / Ctrl+C     Quit"),
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

fn kind_color_style(kind: &NodeKind) -> Style {
    match kind {
        NodeKind::Folder => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        NodeKind::Module => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        NodeKind::File => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        NodeKind::Class => Style::default().fg(Color::Yellow),
        NodeKind::Function => Style::default().fg(Color::LightBlue),
        NodeKind::Block => Style::default().fg(Color::Gray),
        NodeKind::Line => Style::default().fg(Color::Reset),
        NodeKind::SymRef => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::ITALIC),
    }
}

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
