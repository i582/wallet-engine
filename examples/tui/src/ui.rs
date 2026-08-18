//! Ratatui rendering for the example wallet.

use num_bigint::BigUint;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, Wrap};
use wallet_engine::{
    AccountStatus, ActivityDirection, CreatedWallet, Network, ResourcePhase, WalletSnapshot,
};

use crate::app::{App, InputField, Screen};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::Reset;
const SUCCESS: Color = Color::Green;
const WARNING: Color = Color::Yellow;
const DANGER: Color = Color::Red;

pub(crate) fn render(frame: &mut Frame<'_>, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    render_header(frame, header, app);

    match &app.screen {
        Screen::Welcome => render_welcome(frame, body, app),
        Screen::Recovery(created) => render_recovery(frame, body, created),
        Screen::Import => render_import(frame, body, app),
        Screen::Dashboard => render_dashboard(frame, body, app),
        Screen::Send => {
            render_dashboard(frame, body, app);
            render_send_dialog(frame, app);
        }
        Screen::TonConnectLink => {
            render_dashboard(frame, body, app);
            render_ton_connect_link_dialog(frame, app);
        }
        Screen::TonConnectConnecting => {
            render_dashboard(frame, body, app);
            render_ton_connect_connecting_dialog(frame);
        }
        Screen::TonConnectConfirm(prompt) => {
            render_dashboard(frame, body, app);
            render_ton_connect_confirmation(frame, prompt);
        }
        Screen::TonConnectTransaction(prompt) => {
            render_dashboard(frame, body, app);
            render_ton_connect_transaction(frame, prompt);
        }
        Screen::ConfirmDelete => {
            render_dashboard(frame, body, app);
            render_delete_dialog(frame);
        }
    }

    render_footer(frame, footer, app);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " wallet-engine ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  TUI wallet"),
    ]);

    let context = app.descriptor().map_or_else(
        || "TESTNET".to_owned(),
        |descriptor| {
            format!(
                "{}  {}",
                network_label(descriptor.network),
                compact(descriptor.address.as_str(), 20)
            )
        },
    );

    frame.render_widget(
        Paragraph::new(title)
            .block(Block::default().borders(Borders::BOTTOM))
            .alignment(Alignment::Left),
        area,
    );

    let context_width = context.chars().count().min(area.width as usize) as u16;
    if context_width > 0 {
        let context_area = Rect {
            x: area.right().saturating_sub(context_width + 1),
            y: area.y,
            width: context_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(context).style(Style::default().fg(MUTED)),
            context_area,
        );
    }
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let help = match app.screen {
        Screen::Welcome => "[c] create  [i] import  [q] quit",
        Screen::Recovery(_) => "[enter] save wallet  [esc] discard",
        Screen::Import => "[enter] import  [esc] cancel",
        Screen::Dashboard => {
            "[c] copy  [r] refresh  [l] older  [s] send  [t] TON Connect  [x] stop TC  [d] delete"
        }
        Screen::Send => "[tab] next field  [enter] continue/send  [esc] close",
        Screen::TonConnectLink => "[enter] connect  [esc] close · paste a complete tc:// link",
        Screen::TonConnectConnecting => "[esc] cancel TON Connect",
        Screen::TonConnectConfirm(_) | Screen::TonConnectTransaction(_) => {
            "[y] approve  [n/esc] decline"
        }
        Screen::ConfirmDelete => "[y] delete  [n/esc] cancel",
    };

    let mut lines = vec![Line::from(Span::styled(help, Style::default().fg(MUTED)))];
    if let Some(status) = &app.status {
        lines.push(Line::from(vec![
            Span::styled("status: ", Style::default().fg(ACCENT)),
            Span::raw(status),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn render_welcome(frame: &mut Frame<'_>, area: Rect, _app: &App) {
    let [intro, commands] = Layout::vertical([Constraint::Length(6), Constraint::Min(5)])
        .margin(1)
        .areas(area);

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                "Wallet is not configured",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from("Create a new wallet or import 24 recovery words."),
            Line::from(Span::styled(
                "This example uses TON testnet.",
                Style::default().fg(WARNING),
            )),
        ]))
        .block(
            Block::default()
                .title(" Wallet ")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        intro,
    );

    let rows = [
        Row::new([
            "c",
            "Create a wallet",
            "Generate and store a new recovery phrase",
        ]),
        Row::new(["i", "Import a wallet", "Restore from 24 recovery words"]),
        Row::new(["q", "Quit", "Close the application"]),
    ];
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(5),
                Constraint::Length(22),
                Constraint::Min(20),
            ],
        )
        .header(
            Row::new(["KEY", "ACTION", "DESCRIPTION"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .block(Block::default().title(" Commands ").borders(Borders::ALL)),
        commands,
    );
}

fn render_recovery(frame: &mut Frame<'_>, area: Rect, created: &CreatedWallet) {
    let [warning, words, metadata] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(10),
        Constraint::Length(3),
    ])
    .margin(1)
    .areas(area);

    frame.render_widget(
        Paragraph::new("Write these words down in order. Anyone with them can control the wallet.")
            .style(Style::default().fg(WARNING))
            .block(Block::default().title(" Secret ").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        warning,
    );

    let phrase = created
        .recovery_phrase
        .phrase
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    let rows = (0..8).map(|row_index| {
        let cells = (0..3).map(|column_index| {
            let index = row_index + column_index * 8;
            let value = phrase
                .get(index)
                .map_or_else(String::new, |word| format!("{:>2}  {word}", index + 1));
            Cell::from(value)
        });
        Row::new(cells).height(2)
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ],
        )
        .column_spacing(2)
        .block(
            Block::default()
                .title(" Recovery words ")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        words,
    );

    frame.render_widget(
        Paragraph::new(format!(
            "address  {}",
            compact(
                created.descriptor.address.as_str(),
                metadata.width.saturating_sub(11) as usize
            )
        ))
        .style(Style::default().fg(MUTED))
        .block(Block::default().borders(Borders::TOP)),
        metadata,
    );
}

fn render_import(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let content = if app.import_words.is_empty() {
        Span::styled(
            "Type or paste 24 words separated by spaces",
            Style::default().fg(MUTED),
        )
    } else {
        Span::raw(&app.import_words)
    };
    let word_count = app.import_words.split_whitespace().count();

    frame.render_widget(
        Paragraph::new(Line::from(content))
            .block(
                Block::default()
                    .title(format!(" Import · {word_count}/24 "))
                    .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
                    .borders(Borders::ALL)
                    .padding(Padding::uniform(1)),
            )
            .wrap(Wrap { trim: false }),
        area.inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 1,
        }),
    );
}

fn render_dashboard(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let columns = if area.width >= 72 {
        Layout::horizontal([Constraint::Length(32), Constraint::Min(36)])
            .spacing(1)
            .split(area)
    } else {
        Layout::vertical([Constraint::Length(12), Constraint::Min(8)])
            .spacing(1)
            .split(area)
    };

    render_account_pane(frame, columns[0], app);
    render_activity_pane(frame, columns[1], app.snapshot.as_ref());
}

fn render_account_pane(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sections = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(5),
        Constraint::Min(4),
    ])
    .split(area);

    let (balance, status, phase) = app.snapshot.as_ref().map_or_else(
        || ("—".to_owned(), AccountStatus::Unknown, ResourcePhase::Idle),
        |snapshot| {
            let balance = snapshot.account.as_ref().map_or_else(
                || "—".to_owned(),
                |account| format_nanograms(&account.balance_nanograms.to_decimal_string()),
            );
            let status = snapshot
                .account
                .as_ref()
                .map_or(AccountStatus::Unknown, |account| account.status);
            (balance, status, snapshot.account_resource.phase)
        },
    );

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(balance, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(" GRAM", Style::default().fg(MUTED)),
            ]),
            Line::default(),
            Line::from(vec![
                Span::styled(
                    account_status_label(status),
                    Style::default().fg(account_status_color(status)),
                ),
                Span::styled(
                    format!("  {}", resource_phase_label(phase)),
                    Style::default().fg(MUTED),
                ),
            ]),
        ])
        .block(
            Block::default()
                .title(" Account ")
                .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        sections[0],
    );

    let address = app
        .descriptor()
        .map_or("—", |descriptor| descriptor.address.as_str());
    frame.render_widget(
        Paragraph::new(address)
            .block(
                Block::default()
                    .title(" Address ")
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false }),
        sections[1],
    );

    let details = app.snapshot.as_ref().map_or_else(
        || vec![Line::from("revision  —")],
        |snapshot| {
            vec![
                Line::from(format!("revision  {}", snapshot.revision)),
                Line::from(format!(
                    "history   {} item{}",
                    snapshot.activity.len(),
                    if snapshot.activity.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                )),
                Line::from(format!(
                    "more      {}",
                    if snapshot.activity_has_more {
                        "yes"
                    } else {
                        "no"
                    }
                )),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .style(Style::default().fg(MUTED))
            .block(Block::default().title(" State ").borders(Borders::ALL)),
        sections[2],
    );
}

fn render_activity_pane(frame: &mut Frame<'_>, area: Rect, snapshot: Option<&WalletSnapshot>) {
    let header = Row::new(["TYPE", "AMOUNT", "COUNTERPARTY", "TIME"])
        .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .bottom_margin(1);

    let rows = snapshot.into_iter().flat_map(|snapshot| {
        snapshot.activity.iter().map(|item| {
            let (direction, sign, color) = match item.direction {
                ActivityDirection::Received => ("IN", "+", SUCCESS),
                ActivityDirection::Sent => ("OUT", "−", WARNING),
            };
            Row::new(vec![
                Cell::from(Span::styled(direction, Style::default().fg(color))),
                Cell::from(format!(
                    "{sign}{}",
                    format_nanograms(&item.amount_nanograms.to_decimal_string())
                )),
                Cell::from(
                    item.counterparty
                        .as_ref()
                        .map(wallet_engine::TonAddressString::as_str)
                        .map_or_else(|| "—".to_owned(), |value| compact(value, 18)),
                ),
                Cell::from(item.timestamp.to_string()),
            ])
        })
    });

    let title = snapshot.map_or_else(
        || " Activity ".to_owned(),
        |snapshot| {
            format!(
                " Activity · {} ",
                resource_phase_label(snapshot.activity_resource.phase)
            )
        },
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Length(16),
            Constraint::Min(18),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .title(title)
            .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1)),
    );
    frame.render_widget(table, area);

    let empty = snapshot.is_none_or(|snapshot| snapshot.activity.is_empty());
    if empty && area.height > 6 {
        let message = snapshot.map_or("No wallet snapshot", |snapshot| {
            match snapshot.activity_resource.phase {
                ResourcePhase::Failed => "Activity request failed",
                ResourcePhase::Loading => "Loading activity…",
                _ => "No transactions",
            }
        });
        let line_area = Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(4),
            width: area.width.saturating_sub(4),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(MUTED))
                .alignment(Alignment::Center),
            line_area,
        );
    }
}

fn render_send_dialog(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(72, 13, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .title(" Send GRAM ")
            .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL),
        area,
    );

    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let [destination, amount, hint] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(2),
    ])
    .areas(inner);

    render_input(
        frame,
        destination,
        "Destination",
        &app.send_destination,
        app.input_field == InputField::Destination,
    );
    render_input(
        frame,
        amount,
        "Amount",
        &app.send_amount,
        app.input_field == InputField::Amount,
    );
    frame.render_widget(
        Paragraph::new("Enter submits from the amount field.").style(Style::default().fg(MUTED)),
        hint,
    );
}

fn render_input(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str, focused: bool) {
    let border_style = if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };
    frame.render_widget(
        Paragraph::new(value).block(
            Block::default()
                .title(format!(" {title} "))
                .borders(Borders::ALL)
                .border_style(border_style),
        ),
        area,
    );

    if focused {
        let cursor_x = area.x.saturating_add(1).saturating_add(
            value
                .chars()
                .count()
                .min(area.width.saturating_sub(2) as usize) as u16,
        );
        frame.set_cursor_position((cursor_x, area.y.saturating_add(1)));
    }
}

fn render_delete_dialog(frame: &mut Frame<'_>) {
    let area = centered_rect(56, 7, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                "Delete this wallet and its local recovery phrase?",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from(vec![
                Span::styled("[y] delete", Style::default().fg(DANGER)),
                Span::raw("    "),
                Span::styled("[n] cancel", Style::default().fg(MUTED)),
            ]),
        ]))
        .block(
            Block::default()
                .title(" Confirm deletion ")
                .title_style(Style::default().fg(DANGER).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

fn render_ton_connect_link_dialog(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(84, 9, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .title(" TON Connect link ")
            .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL),
        area,
    );
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let [input, hint] = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).areas(inner);
    render_input(
        frame,
        input,
        "tc:// or universal link",
        &app.ton_connect_link,
        true,
    );
    frame.render_widget(
        Paragraph::new("The dApp must remain open while the wallet connects.")
            .style(Style::default().fg(MUTED)),
        hint,
    );
}

fn render_ton_connect_connecting_dialog(frame: &mut Frame<'_>) {
    let area = centered_rect(54, 7, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                "TON Connect",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from("Loading manifest or waiting for bridge traffic…"),
        ]))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::uniform(1)),
        ),
        area,
    );
}

fn render_ton_connect_confirmation(
    frame: &mut Frame<'_>,
    prompt: &crate::ton_connect::ConnectPrompt,
) {
    let area = centered_rect(76, 15, frame.area());
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(
            format!("Connect to {}?", prompt.dapp_name),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(format!("origin   {}", prompt.origin)),
        Line::from(format!("domain   {}", prompt.domain)),
        Line::from(format!("account  {}", compact(&prompt.account, 54))),
        Line::from(format!("icon     {}", compact(&prompt.icon_url, 54))),
    ];
    if let Some(payload) = &prompt.proof_payload {
        lines.push(Line::from(format!("proof    {}", compact(payload, 54))));
    }
    lines.extend([
        Line::default(),
        Line::from(Span::styled(
            "This is an off-chain connection, not a transfer.",
            Style::default().fg(SUCCESS),
        )),
        Line::from("[y] approve    [n] decline"),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .title(" Confirm TON Connect ")
                .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .padding(Padding::uniform(1)),
        ),
        area,
    );
}

fn render_ton_connect_transaction(
    frame: &mut Frame<'_>,
    prompt: &crate::ton_connect::TransactionPrompt,
) {
    let area = centered_rect(70, 14, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                format!("{} requests a transaction", prompt.dapp_name),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from(format!("destination  {}", compact(&prompt.destination, 48))),
            Line::from(format!(
                "amount       {} nanograms",
                prompt.amount_nanograms
            )),
            Line::from(format!(
                "payload      {}",
                if prompt.has_payload {
                    "contract call"
                } else {
                    "empty"
                }
            )),
            Line::from(format!(
                "StateInit    {}",
                if prompt.deploys_contract {
                    "deploy contract"
                } else {
                    "none"
                }
            )),
            Line::default(),
            Line::from(Span::styled(
                "This submits an on-chain transaction and spends network fees.",
                Style::default().fg(WARNING),
            )),
            Line::from("[y] approve and submit    [n] decline"),
        ]))
        .block(
            Block::default()
                .title(" Confirm transaction ")
                .title_style(Style::default().fg(WARNING).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .padding(Padding::uniform(1)),
        ),
        area,
    );
}

fn centered_rect(max_width: u16, height: u16, area: Rect) -> Rect {
    let width = max_width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height.saturating_sub(2)).max(1);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn format_nanograms(value: &str) -> String {
    let Ok(value) = value.parse::<BigUint>() else {
        return "—".to_owned();
    };
    let scale = BigUint::from(1_000_000_000_u64);
    let whole = &value / &scale;
    let remainder = &value % &scale;
    if remainder == BigUint::default() {
        return whole.to_string();
    }

    let fraction = format!("{remainder:0>9}").trim_end_matches('0').to_owned();
    format!("{whole}.{fraction}")
}

fn compact(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    if width < 5 {
        return value.chars().take(width).collect();
    }

    let side = (width - 1) / 2;
    let start = value.chars().take(side).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(side)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}…{end}")
}

const fn network_label(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "MAINNET",
        Network::Testnet => "TESTNET",
    }
}

const fn account_status_label(status: AccountStatus) -> &'static str {
    match status {
        AccountStatus::Nonexistent => "nonexistent",
        AccountStatus::Uninitialized => "uninitialized",
        AccountStatus::Active => "active",
        AccountStatus::Frozen => "frozen",
        AccountStatus::Unknown => "unknown",
    }
}

const fn account_status_color(status: AccountStatus) -> Color {
    match status {
        AccountStatus::Active => SUCCESS,
        AccountStatus::Frozen => DANGER,
        AccountStatus::Uninitialized | AccountStatus::Nonexistent => WARNING,
        AccountStatus::Unknown => MUTED,
    }
}

const fn resource_phase_label(phase: ResourcePhase) -> &'static str {
    match phase {
        ResourcePhase::Idle => "idle",
        ResourcePhase::Loading => "loading",
        ResourcePhase::Ready => "ready",
        ResourcePhase::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::{compact, format_nanograms};

    #[test]
    fn formats_nanograms_without_floating_point() {
        assert_eq!(format_nanograms("0"), "0");
        assert_eq!(format_nanograms("1000000000"), "1");
        assert_eq!(format_nanograms("12340500000"), "12.3405");
        assert_eq!(
            format_nanograms("123456789012345678901234567890"),
            "123456789012345678901.23456789"
        );
    }

    #[test]
    fn compacts_long_values_to_the_requested_width() {
        assert_eq!(compact("short", 9), "short");
        assert_eq!(compact("abcdefghijklmnop", 9), "abcd…mnop");
    }
}
