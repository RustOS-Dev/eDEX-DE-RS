//! Status bar: privacy indicators, network, audio, battery, notifications.

use crate::{
    geometry::{with_alpha, Color, Rect},
    hit::{HitTarget, StatusItem},
    scene::Align,
    state::ShellState,
    widgets::Ctx,
};

struct Indicator {
    text: String,
    color: Color,
    item: StatusItem,
}

pub fn draw(ctx: &mut Ctx, rect: Rect, state: &ShellState) {
    let t = ctx.theme;
    let s = &state.status;
    ctx.scene.fill(rect, t.background);
    ctx.scene.hline(rect.x, rect.bottom() - 1.0, rect.w, with_alpha(t.border, 0.35));
    let size = ctx.small();
    let line = rect.h;
    let cw = ctx.metrics.cell_w * 0.9;

    let on = |b: bool| if b { t.accent } else { t.text_dim };
    let mut left: Vec<Indicator> = vec![
        Indicator { text: format!("TOR {}", s.tor_mode.to_ascii_uppercase()), color: on(s.tor_active), item: StatusItem::Tor },
        Indicator { text: "TAILSCALE".into(), color: on(s.tailscale_active), item: StatusItem::Tailscale },
        Indicator { text: "VPN".into(), color: on(s.vpn_active), item: StatusItem::Vpn },
        Indicator { text: "WG".into(), color: on(s.wireguard_active), item: StatusItem::WireGuard },
        Indicator { text: "FPR".into(), color: on(s.fprintd_active), item: StatusItem::Fingerprint },
    ];
    left.push(Indicator {
        text: if s.mic_active { "MIC ●".into() } else { "MIC".into() },
        color: if s.mic_active { t.error } else { t.text_dim },
        item: StatusItem::Microphone,
    });
    left.push(Indicator {
        text: if s.camera_active { "CAM ●".into() } else { "CAM".into() },
        color: if s.camera_active { t.error } else { t.text_dim },
        item: StatusItem::Camera,
    });

    let mut x = rect.x + 12.0;
    for ind in &left {
        let w = (ind.text.chars().count() as f32 * cw).round() + 8.0;
        let r = Rect::new(x, rect.y, w, line);
        ctx.scene.text_aligned(Rect::new(r.x, r.y + (line - ctx.metrics.line) / 2.0, r.w, ctx.metrics.line), size, ind.color, Align::Left, ind.text.clone());
        ctx.hits.push(r, HitTarget::Status(ind.item));
        x += w + 10.0;
    }

    let net = if let Some(ssid) = &s.wifi_ssid {
        format!("WIFI {ssid}")
    } else if s.ethernet {
        "ETH".to_string()
    } else {
        "OFFLINE".to_string()
    };
    let bt = if s.bluetooth_on { format!("BT {}", s.bluetooth_connected) } else { "BT OFF".to_string() };
    let vol = match s.volume {
        Some(v) if s.muted => format!("VOL {v}% MUTED"),
        Some(v) => format!("VOL {v}%"),
        None => "VOL --".to_string(),
    };
    let bat = match s.battery_pct {
        Some(p) if s.battery_charging => format!("BAT {p}% ⚡"),
        Some(p) => format!("BAT {p}%"),
        None => "AC".to_string(),
    };
    let notif = if s.dnd {
        "DND".to_string()
    } else if s.unread_notifications > 0 {
        format!("NOTIF {}", s.unread_notifications)
    } else {
        "NOTIF".to_string()
    };
    let right: Vec<Indicator> = vec![
        Indicator { text: notif, color: if s.unread_notifications > 0 { t.warning } else { t.text_secondary }, item: StatusItem::Notifications },
        Indicator { text: bat, color: battery_color(t, s.battery_pct, s.battery_charging), item: StatusItem::Battery },
        Indicator { text: vol, color: if s.muted { t.text_dim } else { t.text_secondary }, item: StatusItem::Volume },
        Indicator { text: bt, color: if s.bluetooth_on { t.text_secondary } else { t.text_dim }, item: StatusItem::Network },
        Indicator {
            text: format!("{net}  ▲{:.0} ▼{:.0} kb/s", state.sysinfo.net_tx_kbps, state.sysinfo.net_rx_kbps),
            color: if s.wifi_ssid.is_some() || s.ethernet { t.text_secondary } else { t.warning },
            item: StatusItem::Network,
        },
    ];
    let mut rx = rect.right() - 12.0;
    for ind in &right {
        let w = (ind.text.chars().count() as f32 * cw).round() + 8.0;
        let r = Rect::new(rx - w, rect.y, w, line);
        ctx.scene.text_aligned(Rect::new(r.x, r.y + (line - ctx.metrics.line) / 2.0, r.w, ctx.metrics.line), size, ind.color, Align::Right, ind.text.clone());
        ctx.hits.push(r, HitTarget::Status(ind.item));
        rx -= w + 12.0;
    }
}

fn battery_color(t: &crate::theme::Theme, pct: Option<u8>, charging: bool) -> Color {
    match pct {
        Some(p) if p <= 10 && !charging => t.error,
        Some(p) if p <= 25 && !charging => t.warning,
        Some(_) => t.text_secondary,
        None => t.text_dim,
    }
}
