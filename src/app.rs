use crate::ollama::{
    self, format_bytes, format_context, InstalledModel, ModelParams, OllamaSnapshot, RunningModel,
    ServiceState,
};
use crate::telemetry::{
    self, format_tokens, format_tokens_compact, DailyTokens, IngestOutcome, TelemetryEvent,
    TelemetryState, UsageStore,
};
use chrono::{DateTime, Local, Utc};
use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::padded_control;
use cosmic::iced::alignment::{Alignment, Horizontal, Vertical};
use cosmic::iced::platform_specific::shell::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::window::Id;
use cosmic::iced::{Border, Color, Length, Limits, Shadow, Subscription};
use cosmic::theme;
use cosmic::widget::{
    self, autosize, button, column, container, divider, icon, row, scrollable, slider, text,
    text_input,
};
use cosmic::{Action, Application, Element};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

const APP_ID: &str = "dev.ashraf.CosmicAppletOllama";
const POPUP_WIDTH: f32 = 400.0;
const HBAR_MAX_WIDTH: f32 = 224.0;
const HBAR_HEIGHT: f32 = 14.0;
const INPUT_BAR_COLOR: Color = Color::from_rgb(0.24, 0.73, 0.78);
const CACHED_BAR_COLOR: Color = Color::from_rgb(0.60, 0.47, 0.95);
const OUTPUT_BAR_COLOR: Color = Color::from_rgb(0.96, 0.66, 0.24);
const ICON_SVG: &[u8] = include_bytes!("../res/icons/dev.ashraf.CosmicAppletOllama-symbolic.svg");
static POPUP_AUTOSIZE_ID: LazyLock<widget::Id> =
    LazyLock::new(|| widget::Id::new("cosmic-applet-autosize"));

#[derive(Debug, Clone)]
pub struct ConfigDraft {
    pub source_model: String,
    pub display_name: String,
    pub max_context: Option<u64>,
    pub was_running: bool,
    pub loading_params: bool,
    pub saving: bool,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: f32,
    pub repeat_penalty: f32,
    pub num_ctx_input: String,
    pub num_predict_input: String,
    pub save_as_tag: String,
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ConfigDraft {
    fn from_params(
        source_model: String,
        max_context: Option<u64>,
        was_running: bool,
        loading_params: bool,
        params: &ModelParams,
    ) -> Self {
        Self {
            display_name: ollama::compact_model_name(&source_model),
            source_model,
            max_context,
            was_running,
            loading_params,
            saving: false,
            temperature: params.temperature.unwrap_or(0.70).clamp(0.0, 2.0),
            top_p: params.top_p.unwrap_or(0.95).clamp(0.0, 1.0),
            top_k: (params.top_k.unwrap_or(40).clamp(1, 150)) as f32,
            repeat_penalty: params.repeat_penalty.unwrap_or(1.10).clamp(0.8, 2.0),
            num_ctx_input: params.num_ctx.map(|v| v.to_string()).unwrap_or_default(),
            num_predict_input: params
                .num_predict
                .filter(|v| *v > 0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            save_as_tag: String::new(),
            extra: params.extra.clone(),
        }
    }

    fn apply_fetched(&mut self, params: ModelParams) {
        self.loading_params = false;
        self.temperature = params.temperature.unwrap_or(0.70).clamp(0.0, 2.0);
        self.top_p = params.top_p.unwrap_or(0.95).clamp(0.0, 1.0);
        self.top_k = (params.top_k.unwrap_or(40).clamp(1, 150)) as f32;
        self.repeat_penalty = params.repeat_penalty.unwrap_or(1.10).clamp(0.8, 2.0);
        self.num_ctx_input = params.num_ctx.map(|v| v.to_string()).unwrap_or_default();
        self.num_predict_input = params
            .num_predict
            .filter(|v| *v > 0)
            .map(|v| v.to_string())
            .unwrap_or_default();
        self.extra = params.extra;
    }

    fn build_model_params(&self) -> Result<ModelParams, String> {
        let num_ctx = if self.num_ctx_input.trim().is_empty() {
            None
        } else {
            let parsed: u64 = self
                .num_ctx_input
                .trim()
                .parse()
                .map_err(|_| "Context window (num_ctx) must be a positive integer".to_string())?;
            if parsed < 128 {
                return Err("Context window (num_ctx) must be at least 128 tokens".to_string());
            }
            Some(parsed)
        };

        let num_predict = if self.num_predict_input.trim().is_empty()
            || self.num_predict_input.trim() == "-1"
        {
            None
        } else {
            let parsed: i32 = self
                .num_predict_input
                .trim()
                .parse()
                .map_err(|_| "Max output (num_predict) must be an integer".to_string())?;
            Some(parsed)
        };

        Ok(ModelParams {
            temperature: Some(self.temperature),
            top_p: Some(self.top_p),
            top_k: Some(self.top_k.round() as i32),
            repeat_penalty: Some(self.repeat_penalty),
            num_ctx,
            num_predict,
            extra: self.extra.clone(),
        })
    }

    fn target_model_name(&self) -> String {
        let custom = self.save_as_tag.trim();
        if custom.is_empty() {
            self.source_model.clone()
        } else if let Some(suffix) = custom.strip_prefix(':') {
            let base = self
                .source_model
                .split(':')
                .next()
                .unwrap_or(&self.source_model);
            format!("{base}:{suffix}")
        } else {
            custom.to_string()
        }
    }
}

#[derive(Debug, Clone)]
pub enum PopupView {
    Overview,
    History { confirm_clear: bool },
    Configure(ConfigDraft),
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    RefreshFull,
    PollPsOnly,
    SnapshotLoaded(OllamaSnapshot),
    TelemetryInitialized(TelemetryState),
    TelemetryStreamEvent(TelemetryEvent),
    UsageStoreSaved,
    OpenHistory,
    RequestClearHistory,
    ConfirmClearHistory,
    HistoryCleared(UsageStore),
    BackToOverview,
    TickNow,
    UnloadModel(String),
    LoadModel(String),
    SetKeepAlive(String, serde_json::Value),
    ActionCompleted(Result<String, String>),
    OpenConfigure(String),
    ModelParamsLoaded(Result<(String, ModelParams), String>),
    ConfigTempChanged(f32),
    ConfigTopPChanged(f32),
    ConfigTopKChanged(f32),
    ConfigRepeatChanged(f32),
    ConfigNumCtxChanged(String),
    ConfigNumPredictChanged(String),
    ConfigSaveAsChanged(String),
    SaveConfiguration,
    ConfigurationSaved(Result<String, String>),
    CopyEndpoint,
    ClearCopiedFeedback(u64),
}

pub struct OllamaApplet {
    core: Core,
    popup: Option<Id>,
    view_mode: PopupView,
    snapshot: OllamaSnapshot,
    telemetry: TelemetryState,
    now: DateTime<Utc>,
    refreshing: bool,
    busy_models: HashSet<String>,
    status_banner: Option<String>,
    last_error: Option<String>,
    endpoint_copied: bool,
    endpoint_copied_seq: u64,
}

impl OllamaApplet {
    fn panel_label(&self) -> String {
        match &self.snapshot.state {
            ServiceState::Offline { .. } => "Offline".to_string(),
            ServiceState::Online { .. } => {
                if self.snapshot.running.is_empty() {
                    "Idle".to_string()
                } else if self.snapshot.running.len() == 1 {
                    self.snapshot.running[0].display_name.clone()
                } else {
                    format!(
                        "{} +{}",
                        self.snapshot.running[0].display_name,
                        self.snapshot.running.len() - 1
                    )
                }
            }
        }
    }

    fn symbolic_icon(&self, size: u16) -> cosmic::widget::Icon {
        let handle = icon::from_svg_bytes(ICON_SVG).symbolic(true);
        widget::icon(handle)
            .class(theme::Svg::Custom(Rc::new(|theme| {
                cosmic::iced::widget::svg::Style {
                    color: Some(theme.cosmic().background(theme.transparent).on.into()),
                }
            })))
            .width(Length::Fixed(size as f32))
            .height(Length::Fixed(size as f32))
    }

    fn popup_icon(&self, size: u16) -> cosmic::widget::Icon {
        let handle = icon::from_svg_bytes(ICON_SVG).symbolic(true);
        widget::icon(handle)
            .class(theme::Svg::Custom(Rc::new(|theme| {
                cosmic::iced::widget::svg::Style {
                    color: Some(theme.cosmic().background(false).on.into()),
                }
            })))
            .width(Length::Fixed(size as f32))
            .height(Length::Fixed(size as f32))
    }

    /// Solid (100% opaque) popup container
    fn solid_popup_container<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let (vertical_align, horizontal_align) = match self.core.applet.anchor {
            PanelAnchor::Left => (Vertical::Center, Horizontal::Left),
            PanelAnchor::Right => (Vertical::Center, Horizontal::Right),
            PanelAnchor::Top => (Vertical::Top, Horizontal::Center),
            PanelAnchor::Bottom => (Vertical::Bottom, Horizontal::Center),
        };

        let styled_inner = container(content).class(theme::Container::Custom(Box::new(|theme| {
            let cosmic = theme.cosmic();
            let corners = cosmic.corner_radii;
            let palette = cosmic.background(false);
            let mut bg: Color = palette.base.into();
            bg.a = 1.0;
            let mut border_color: Color = palette.divider.into();
            border_color.a = 1.0;

            cosmic::iced::widget::container::Style {
                text_color: Some(palette.on.into()),
                background: Some(bg.into()),
                border: Border {
                    radius: corners.radius_m.into(),
                    width: 1.0,
                    color: border_color,
                },
                shadow: Shadow::default(),
                icon_color: Some(palette.on.into()),
                snap: true,
            }
        })));

        autosize::autosize(
            container(styled_inner)
                .height(Length::Shrink)
                .align_x(horizontal_align)
                .align_y(vertical_align),
            POPUP_AUTOSIZE_ID.clone(),
        )
        .limits(
            Limits::NONE
                .min_height(1.0)
                .min_width(POPUP_WIDTH)
                .max_width(POPUP_WIDTH)
                .max_height(1000.0),
        )
        .into()
    }

    fn render_today_token_card<'a>(&self) -> Element<'a, Message> {
        let today_date = Local::now().format("%b %d").to_string();
        let today = self.telemetry.today_usage();

        let section_header = row([])
            .align_y(Alignment::Center)
            .spacing(8)
            .push(
                container(text::caption(format!("TODAY'S TOKEN USAGE · {today_date}")))
                    .width(Length::Fill),
            )
            .push(
                button::text("History").on_press(Message::OpenHistory),
            );

        let in_str = if today.cached_input_tokens > 0 {
            format!(
                "{} ({} cached)",
                format_tokens(today.input_tokens),
                format_tokens(today.cached_input_tokens)
            )
        } else {
            format_tokens(today.input_tokens)
        };

        let out_str = format_tokens(today.output_tokens);
        let req_word = if today.requests == 1 { "req" } else { "reqs" };
        let total_str = format!("{} · {} {}", format_tokens(today.total()), today.requests, req_word);

        let mut rows_col = column([])
            .spacing(4)
            .push(kv_row("Input tokens", in_str))
            .push(kv_row("Output tokens", out_str))
            .push(kv_row("Total today", total_str));

        if let Some(live) = &self.telemetry.active_generation {
            let speed_part = live
                .current_tok_per_sec
                .map(|r| format!(" · {r:.1} tok/s"))
                .unwrap_or_default();
            rows_col = rows_col.push(kv_row(
                "Active stream",
                format!(
                    "{} in · {} out{speed_part}",
                    format_tokens(live.prompt_tokens),
                    format_tokens(live.generated_so_far)
                ),
            ));
        } else if let Some(tps) = self.telemetry.last_tok_per_sec {
            rows_col = rows_col.push(kv_row(
                "Last request",
                format!(
                    "{} in · {} out · {tps:.1} tok/s",
                    format_tokens(self.telemetry.last_input_tokens),
                    format_tokens(self.telemetry.last_output_tokens)
                ),
            ));
        }

        column([])
            .spacing(6)
            .push(section_header)
            .push(solid_card(rows_col))
            .into()
    }

    fn render_running_card<'a>(&self, model: &'a RunningModel) -> Element<'a, Message> {
        let is_busy = self.busy_models.contains(&model.name);
        let cfg_btn =
            button::standard("Configure").on_press(Message::OpenConfigure(model.name.clone()));
        let unload_btn = if is_busy {
            button::standard("Working…")
        } else {
            button::destructive("Unload").on_press(Message::UnloadModel(model.name.clone()))
        };

        let title_row = row([])
            .align_y(Alignment::Center)
            .spacing(6)
            .push(
                column([])
                    .spacing(2)
                    .width(Length::Fill)
                    .push(text::heading(&model.display_name))
                    .push(text::caption(&model.name)),
            )
            .push(cfg_btn)
            .push(unload_btn);

        let mut specs_line_1 = Vec::new();
        if !model.family.is_empty() {
            specs_line_1.push(model.family.clone());
        }
        if !model.parameter_size.is_empty() {
            specs_line_1.push(model.parameter_size.clone());
        }
        if !model.quantization.is_empty() {
            specs_line_1.push(model.quantization.clone());
        }
        if !model.runner.is_empty() {
            specs_line_1.push(model.runner.clone());
        }

        let mem_value = if model.size_vram_bytes > 0 {
            format!(
                "{} ({} VRAM)",
                format_bytes(model.size_bytes),
                format_bytes(model.size_vram_bytes)
            )
        } else {
            format!("{} RAM", format_bytes(model.size_bytes))
        };

        let ctx_value = match (model.context_length, model.max_context_length) {
            (Some(active), Some(max)) if max > active => {
                format!("{} (max {})", format_context(active), format_context(max))
            }
            (Some(active), _) => format_context(active),
            (None, Some(max)) => format!("up to {}", format_context(max)),
            (None, None) => "Default".to_string(),
        };

        let mut details_col = column([]).spacing(4);
        if !specs_line_1.is_empty() {
            details_col = details_col.push(kv_row("Architecture", specs_line_1.join(" · ")));
        }
        details_col = details_col
            .push(kv_row("Memory", mem_value))
            .push(kv_row("Processor", model.processor_summary()))
            .push(kv_row("Context", ctx_value))
            .push(kv_row("Parameters", model.params.summary_line()));

        if !model.capabilities.is_empty() {
            details_col = details_col.push(kv_row("Capabilities", model.capabilities.join(", ")));
        }
        details_col = details_col.push(kv_row("Auto-unload", model.expiry_summary(self.now)));

        // Keep-alive quick residency buttons
        let pinned = model.is_pinned_forever(self.now);
        let keep_row = row([])
            .align_y(Alignment::Center)
            .spacing(4)
            .push(container(text::caption("Residency")).width(Length::Fill))
            .push(keep_alive_btn(&model.name, "5m", serde_json::Value::from("5m"), false, is_busy))
            .push(keep_alive_btn(&model.name, "15m", serde_json::Value::from("15m"), false, is_busy))
            .push(keep_alive_btn(&model.name, "1h", serde_json::Value::from("1h"), false, is_busy))
            .push(keep_alive_btn(&model.name, "Pin ∞", serde_json::Value::from(-1), pinned, is_busy));

        let card_inner = column([])
            .spacing(8)
            .push(title_row)
            .push(divider::horizontal::light())
            .push(details_col)
            .push(divider::horizontal::light())
            .push(keep_row);

        solid_card(card_inner)
    }

    fn render_installed_row<'a>(
        &self,
        model: &'a InstalledModel,
        is_running: bool,
    ) -> Element<'a, Message> {
        let is_busy = self.busy_models.contains(&model.name);

        let mut meta_parts = Vec::new();
        if !model.parameter_size.is_empty() {
            meta_parts.push(model.parameter_size.clone());
        }
        if !model.quantization.is_empty() {
            meta_parts.push(model.quantization.clone());
        }
        if model.size_bytes > 0 {
            meta_parts.push(format_bytes(model.size_bytes));
        }
        if let Some(ctx) = model.context_length {
            meta_parts.push(format!("{} ctx", format_context(ctx)));
        }

        let info_col = column([])
            .spacing(2)
            .width(Length::Fill)
            .push(text::body(&model.display_name))
            .push(text::caption(meta_parts.join(" · ")));

        let cfg_btn = button::text("Configure").on_press(Message::OpenConfigure(model.name.clone()));

        let action_widget: Element<'a, Message> = if is_running {
            text::caption("Active").into()
        } else if is_busy {
            button::standard("Loading…").into()
        } else {
            button::standard("Load")
                .on_press(Message::LoadModel(model.name.clone()))
                .into()
        };

        row([])
            .align_y(Alignment::Center)
            .spacing(6)
            .push(info_col)
            .push(cfg_btn)
            .push(action_widget)
            .into()
    }

    fn render_overview<'a>(&'a self) -> Element<'a, Message> {
        let status_subtitle = match &self.snapshot.state {
            ServiceState::Online { version } => {
                if version.is_empty() {
                    "Service online".to_string()
                } else {
                    format!("Online · v{version}")
                }
            }
            ServiceState::Offline { systemd_state, .. } => {
                format!("Offline · systemd: {systemd_state}")
            }
        };

        let header_left = row([])
            .align_y(Alignment::Center)
            .spacing(8)
            .width(Length::Fill)
            .push(self.popup_icon(22))
            .push(
                column([])
                    .spacing(1)
                    .push(text::title4("Ollama"))
                    .push(text::caption(status_subtitle)),
            );

        let refresh_btn = button::icon(icon::from_name("view-refresh-symbolic"))
            .extra_small()
            .on_press(Message::RefreshFull);

        let header = padded_control(
            row([])
                .align_y(Alignment::Center)
                .spacing(8)
                .push(header_left)
                .push(refresh_btn),
        );

        let mut body = column([]).spacing(8).padding([8, 0]).push(header);
        body = body.push(padded_control(divider::horizontal::default()));

        if let Some(banner) = &self.status_banner {
            body = body.push(padded_control(text::caption(banner)));
        }

        // Daily token usage card
        body = body.push(padded_control(self.render_today_token_card()));
        body = body.push(padded_control(divider::horizontal::light()));

        match &self.snapshot.state {
            ServiceState::Offline {
                systemd_state,
                error,
            } => {
                let offline_box = column([])
                    .spacing(6)
                    .push(text::body(format!("Cannot reach Ollama ({error})")))
                    .push(text::caption(format!("ollama.service is {systemd_state}")))
                    .push(text::caption("Run: sudo systemctl start ollama"));
                body = body.push(padded_control(offline_box));
            }
            ServiceState::Online { .. } => {
                let active_header = row([])
                    .align_y(Alignment::Center)
                    .push(container(text::caption("LOADED IN MEMORY")).width(Length::Fill))
                    .push(text::caption(format!("{} active", self.snapshot.running.len())));
                body = body.push(padded_control(active_header));

                if self.snapshot.running.is_empty() {
                    let idle_card = solid_card(
                        column([])
                            .spacing(4)
                            .push(text::body("No model currently loaded"))
                            .push(text::caption(
                                "Ollama is idle (0 VRAM). Models load automatically on inference or via Load below.",
                            )),
                    );
                    body = body.push(padded_control(idle_card));
                } else {
                    let mut active_col = column([]).spacing(8);
                    for running in &self.snapshot.running {
                        active_col = active_col.push(self.render_running_card(running));
                    }
                    body = body.push(padded_control(active_col));
                }

                if !self.snapshot.installed.is_empty() {
                    body = body.push(padded_control(divider::horizontal::light()));
                    let installed_header = row([])
                        .align_y(Alignment::Center)
                        .push(container(text::caption("INSTALLED MODELS")).width(Length::Fill))
                        .push(text::caption(format!(
                            "{} total",
                            self.snapshot.installed.len()
                        )));
                    body = body.push(padded_control(installed_header));

                    let running_names: HashSet<&str> = self
                        .snapshot
                        .running
                        .iter()
                        .map(|m| m.name.as_str())
                        .collect();

                    let mut list_col = column([]).spacing(8);
                    for inst in &self.snapshot.installed {
                        let is_running = running_names.contains(inst.name.as_str());
                        list_col = list_col.push(self.render_installed_row(inst, is_running));
                    }

                    let list_scroll = scrollable(padded_control(list_col)).height(Length::Shrink);
                    body = body.push(container(list_scroll).max_height(180.0));
                }
            }
        }

        if let Some(err) = &self.last_error {
            body = body.push(padded_control(divider::horizontal::light()));
            body = body.push(padded_control(text::caption(format!("Error: {err}"))));
        }

        body = body.push(padded_control(divider::horizontal::light()));
        body = body.push(self.render_endpoint_row());

        body.into()
    }

    fn render_endpoint_row<'a>(&'a self) -> Element<'a, Message> {
        let (right_label, icon_name) = if self.endpoint_copied {
            (
                format!("{} · Copied!", self.snapshot.host),
                "object-select-symbolic",
            )
        } else {
            (self.snapshot.host.clone(), "edit-copy-symbolic")
        };

        let copy_icon = icon::from_name(icon_name).size(14).symbolic(true);
        let row_inner = row([])
            .align_y(Alignment::Center)
            .spacing(6)
            .width(Length::Fill)
            .push(container(text::caption("Endpoint")).width(Length::Fill))
            .push(text::caption(right_label))
            .push(widget::icon(copy_icon.into()).size(14));

        cosmic::applet::menu_button(row_inner)
            .on_press(Message::CopyEndpoint)
            .into()
    }

    fn render_stacked_bar_chart<'a>(&'a self) -> Element<'a, Message> {
        let series = self.telemetry.history_bars_series(7);
        let max_total = series.iter().map(|(_, _, t)| t.total()).max().unwrap_or(0);
        let num_rows = series.len();

        let chart_header = row([])
            .align_y(Alignment::Center)
            .spacing(8)
            .push(container(text::caption("DAILY USAGE")).width(Length::Fill))
            .push(
                row([])
                    .align_y(Alignment::Center)
                    .spacing(8)
                    .push(legend_item(INPUT_BAR_COLOR, "Input"))
                    .push(legend_item(CACHED_BAR_COLOR, "Cached"))
                    .push(legend_item(OUTPUT_BAR_COLOR, "Output")),
            );

        let mut bars_col = column([]).spacing(8).width(Length::Fill);

        for (_date_key, short_label, tokens) in series {
            let day_total = tokens.total();
            let segments = build_horizontal_segments(&tokens, max_total, HBAR_MAX_WIDTH);
            let used_w: f32 = segments.iter().map(|(w, _, _)| *w).sum();

            let mut stack_row = row([]).width(Length::Fill).height(Length::Fill);
            for (seg_w, seg_color, radii) in segments {
                stack_row = stack_row.push(colored_segment(
                    Length::Fixed(seg_w),
                    Length::Fill,
                    seg_color,
                    radii,
                ));
            }
            let empty_w = (HBAR_MAX_WIDTH - used_w).max(0.0);
            if empty_w > 0.0 {
                stack_row = stack_row.push(
                    container(row([]))
                        .width(Length::Fixed(empty_w))
                        .height(Length::Fill),
                );
            }

            let bar_track = container(stack_row)
                .width(Length::Fixed(HBAR_MAX_WIDTH))
                .height(Length::Fixed(HBAR_HEIGHT))
                .class(theme::Container::Custom(Box::new(|_theme| {
                    cosmic::iced::widget::container::Style {
                        text_color: None,
                        background: Some(Color::from_rgba(0.5, 0.5, 0.5, 0.14).into()),
                        border: Border {
                            radius: 4.0.into(),
                            width: 0.0,
                            color: Color::TRANSPARENT,
                        },
                        shadow: Shadow::default(),
                        icon_color: None,
                        snap: true,
                    }
                })));

            let right_text = if day_total > 0 {
                format_tokens_compact(day_total)
            } else {
                "—".to_string()
            };

            let day_row = row([])
                .align_y(Alignment::Center)
                .spacing(8)
                .width(Length::Fill)
                .push(container(text::caption(short_label)).width(Length::Fixed(52.0)))
                .push(bar_track)
                .push(
                    container(text::caption(right_text))
                        .width(Length::Fill)
                        .align_x(Horizontal::Right),
                );

            bars_col = bars_col.push(day_row);
        }

        let bars_element: Element<'a, Message> = if num_rows > 10 {
            container(scrollable(bars_col)).max_height(260.0).into()
        } else {
            bars_col.into()
        };

        let last_7 = self.telemetry.last_n_days_total(7);
        let all_time = self.telemetry.all_time_total();
        let cached_7 = last_7.cached_input_tokens.min(last_7.input_tokens);
        let uncached_7 = last_7.input_tokens.saturating_sub(cached_7);

        let breakdown_7_row = row([])
            .align_y(Alignment::Center)
            .spacing(12)
            .push(legend_stat(
                INPUT_BAR_COLOR,
                format!("{} in", format_tokens(uncached_7)),
            ))
            .push(legend_stat(
                CACHED_BAR_COLOR,
                format!("{} cached", format_tokens(cached_7)),
            ))
            .push(legend_stat(
                OUTPUT_BAR_COLOR,
                format!("{} out", format_tokens(last_7.output_tokens)),
            ));

        let req_7_word = if last_7.requests == 1 { "req" } else { "reqs" };
        let req_all_word = if all_time.requests == 1 { "req" } else { "reqs" };

        solid_card(
            column([])
                .spacing(10)
                .push(chart_header)
                .push(bars_element)
                .push(divider::horizontal::light())
                .push(breakdown_7_row)
                .push(
                    column([])
                        .spacing(4)
                        .push(kv_row(
                            "Last 7 days",
                            format!(
                                "{} · {} {}",
                                format_tokens(last_7.total()),
                                last_7.requests,
                                req_7_word
                            ),
                        ))
                        .push(kv_row(
                            "All-time",
                            format!(
                                "{} · {} {}",
                                format_tokens(all_time.total()),
                                all_time.requests,
                                req_all_word
                            ),
                        )),
                ),
        )
    }

    fn render_history<'a>(&'a self, confirm_clear: bool) -> Element<'a, Message> {
        let clear_widget: Element<'a, Message> = if confirm_clear {
            button::destructive("Confirm Clear").on_press(Message::ConfirmClearHistory).into()
        } else {
            button::text("Clear").on_press(Message::RequestClearHistory).into()
        };

        let header = padded_control(
            row([])
                .align_y(Alignment::Center)
                .spacing(8)
                .push(button::standard("Back").on_press(Message::BackToOverview))
                .push(container(text::title4("Daily Token Usage")).width(Length::Fill))
                .push(clear_widget),
        );

        let chart_card = self.render_stacked_bar_chart();

        column([])
            .spacing(8)
            .padding([8, 0])
            .push(header)
            .push(padded_control(divider::horizontal::default()))
            .push(padded_control(chart_card))
            .into()
    }

    fn render_configure<'a>(&'a self, draft: &'a ConfigDraft) -> Element<'a, Message> {
        let header = padded_control(
            row([])
                .align_y(Alignment::Center)
                .spacing(8)
                .push(button::standard("Back").on_press(Message::BackToOverview))
                .push(
                    column([])
                        .spacing(1)
                        .width(Length::Fill)
                        .push(text::title4(format!("Configure {}", draft.display_name)))
                        .push(text::caption(&draft.source_model)),
                ),
        );

        if draft.loading_params {
            return column([])
                .spacing(12)
                .padding([8, 0])
                .push(header)
                .push(padded_control(divider::horizontal::default()))
                .push(padded_control(text::body("Reading current Modelfile parameters…")))
                .into();
        }

        let temp_ctrl = column([])
            .spacing(4)
            .push(kv_row("Temperature", format!("{:.2}", draft.temperature)))
            .push(
                slider(0.0..=2.0, draft.temperature, Message::ConfigTempChanged).step(0.05_f32),
            );

        let top_p_ctrl = column([])
            .spacing(4)
            .push(kv_row("Top P", format!("{:.2}", draft.top_p)))
            .push(slider(0.0..=1.0, draft.top_p, Message::ConfigTopPChanged).step(0.05_f32));

        let top_k_ctrl = column([])
            .spacing(4)
            .push(kv_row("Top K", format!("{}", draft.top_k.round() as i32)))
            .push(slider(1.0..=150.0, draft.top_k, Message::ConfigTopKChanged).step(1.0_f32));

        let rep_ctrl = column([])
            .spacing(4)
            .push(kv_row(
                "Repeat Penalty",
                format!("{:.2}", draft.repeat_penalty),
            ))
            .push(
                slider(0.80..=2.0, draft.repeat_penalty, Message::ConfigRepeatChanged)
                    .step(0.05_f32),
            );

        // Context size presets
        let max_ctx = draft.max_context.unwrap_or(262_144);
        let presets: &[(u64, &str)] = &[
            (4096, "4K"),
            (8192, "8K"),
            (16384, "16K"),
            (32768, "32K"),
            (65536, "64K"),
            (131072, "128K"),
            (262144, "256K"),
        ];
        let mut ctx_chips = row([])
            .spacing(4)
            .align_y(Alignment::Center)
            .push(preset_chip(
                "Auto",
                draft.num_ctx_input.trim().is_empty(),
                Message::ConfigNumCtxChanged(String::new()),
            ));
        for &(val, label) in presets {
            if val <= max_ctx {
                let selected = draft.num_ctx_input.trim() == val.to_string();
                ctx_chips = ctx_chips.push(preset_chip(
                    label,
                    selected,
                    Message::ConfigNumCtxChanged(val.to_string()),
                ));
            }
        }

        let ctx_hint = draft
            .max_context
            .map(|m| format!("Context Window · num_ctx (max {})", format_context(m)))
            .unwrap_or_else(|| "Context Window · num_ctx".to_string());

        let ctx_ctrl = column([])
            .spacing(6)
            .push(text::caption(ctx_hint))
            .push(ctx_chips)
            .push(
                text_input("Default (or enter token count e.g. 8192)", &draft.num_ctx_input)
                    .on_input(Message::ConfigNumCtxChanged),
            );

        // Max output presets
        let out_chips = row([])
            .spacing(4)
            .align_y(Alignment::Center)
            .push(preset_chip(
                "Unlimited",
                draft.num_predict_input.trim().is_empty(),
                Message::ConfigNumPredictChanged(String::new()),
            ))
            .push(preset_chip(
                "512",
                draft.num_predict_input.trim() == "512",
                Message::ConfigNumPredictChanged("512".to_string()),
            ))
            .push(preset_chip(
                "2K",
                draft.num_predict_input.trim() == "2048",
                Message::ConfigNumPredictChanged("2048".to_string()),
            ))
            .push(preset_chip(
                "4K",
                draft.num_predict_input.trim() == "4096",
                Message::ConfigNumPredictChanged("4096".to_string()),
            ))
            .push(preset_chip(
                "8K",
                draft.num_predict_input.trim() == "8192",
                Message::ConfigNumPredictChanged("8192".to_string()),
            ));

        let out_ctrl = column([])
            .spacing(6)
            .push(text::caption("Max Output Tokens · num_predict"))
            .push(out_chips)
            .push(
                text_input("Unlimited (-1)", &draft.num_predict_input)
                    .on_input(Message::ConfigNumPredictChanged),
            );

        let save_target_preview = draft.target_model_name();
        let save_as_ctrl = column([])
            .spacing(4)
            .push(text::caption(format!(
                "Save Target (will save to: {save_target_preview})"
            )))
            .push(
                text_input(
                    "Blank = update in-place, or e.g. :coding",
                    &draft.save_as_tag,
                )
                .on_input(Message::ConfigSaveAsChanged),
            );

        let form_col = column([])
            .spacing(12)
            .push(solid_card(
                column([])
                    .spacing(10)
                    .push(temp_ctrl)
                    .push(top_p_ctrl)
                    .push(top_k_ctrl)
                    .push(rep_ctrl),
            ))
            .push(solid_card(
                column([])
                    .spacing(10)
                    .push(ctx_ctrl)
                    .push(divider::horizontal::light())
                    .push(out_ctrl),
            ))
            .push(solid_card(save_as_ctrl));

        let save_btn: Element<'a, Message> = if draft.saving {
            button::suggested("Saving…").into()
        } else {
            button::suggested("Save Parameters")
                .on_press(Message::SaveConfiguration)
                .into()
        };

        let mut footer_col = column([]).spacing(6);
        if let Some(err) = &self.last_error {
            footer_col = footer_col.push(text::caption(format!("Error: {err}")));
        }
        footer_col = footer_col.push(
            row([])
                .align_y(Alignment::Center)
                .spacing(8)
                .push(
                    container(button::standard("Cancel").on_press(Message::BackToOverview))
                        .width(Length::Fill),
                )
                .push(save_btn),
        );

        let scroll_form = scrollable(padded_control(form_col)).height(Length::Shrink);

        column([])
            .spacing(8)
            .padding([8, 0])
            .push(header)
            .push(padded_control(divider::horizontal::default()))
            .push(container(scroll_form).max_height(430.0))
            .push(padded_control(divider::horizontal::light()))
            .push(padded_control(footer_col))
            .into()
    }
}

fn colored_segment<'a>(
    width: Length,
    height: Length,
    color: Color,
    radii: [f32; 4],
) -> Element<'a, Message> {
    container(row([]))
        .width(width)
        .height(height)
        .class(theme::Container::Custom(Box::new(move |_theme| {
            cosmic::iced::widget::container::Style {
                text_color: None,
                background: Some(color.into()),
                border: Border {
                    radius: radii.into(),
                    width: 0.0,
                    color: Color::TRANSPARENT,
                },
                shadow: Shadow::default(),
                icon_color: None,
                snap: true,
            }
        })))
        .into()
}

fn legend_item<'a>(color: Color, label: &'static str) -> Element<'a, Message> {
    row([])
        .align_y(Alignment::Center)
        .spacing(4)
        .push(colored_segment(
            Length::Fixed(8.0),
            Length::Fixed(8.0),
            color,
            [2.0, 2.0, 2.0, 2.0],
        ))
        .push(text::caption(label))
        .into()
}

fn legend_stat<'a>(color: Color, label: String) -> Element<'a, Message> {
    row([])
        .align_y(Alignment::Center)
        .spacing(4)
        .push(colored_segment(
            Length::Fixed(8.0),
            Length::Fixed(8.0),
            color,
            [2.0, 2.0, 2.0, 2.0],
        ))
        .push(text::caption(label))
        .into()
}

fn build_horizontal_segments(
    tokens: &DailyTokens,
    max_total: u64,
    max_width: f32,
) -> Vec<(f32, Color, [f32; 4])> {
    let day_total = tokens.total();
    if day_total == 0 || max_total == 0 {
        return Vec::new();
    }

    let cached = tokens.cached_input_tokens.min(tokens.input_tokens);
    let uncached_in = tokens.input_tokens.saturating_sub(cached);
    let out = tokens.output_tokens;

    let mut parts: Vec<(u64, Color)> = Vec::with_capacity(3);
    if uncached_in > 0 {
        parts.push((uncached_in, INPUT_BAR_COLOR));
    }
    if cached > 0 {
        parts.push((cached, CACHED_BAR_COLOR));
    }
    if out > 0 {
        parts.push((out, OUTPUT_BAR_COLOR));
    }
    if parts.is_empty() {
        return Vec::new();
    }

    let min_bar_w = (parts.len() as f32 * 3.0).max(6.0);
    let raw_w = (day_total as f32 / max_total as f32) * max_width;
    let total_w = raw_w.clamp(min_bar_w, max_width).round();

    let mut widths: Vec<f32> = parts
        .iter()
        .map(|(count, _)| ((*count as f32 / day_total as f32) * total_w).round().max(3.0))
        .collect();

    let sum_w: f32 = widths.iter().sum();
    let diff = total_w - sum_w;
    if diff.abs() > 0.01 {
        if let Some((max_idx, _)) = widths
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            widths[max_idx] = (widths[max_idx] + diff).max(3.0);
        }
    }

    let len = parts.len();
    parts
        .into_iter()
        .zip(widths)
        .enumerate()
        .map(|(idx, ((_, color), w))| {
            let left_r = if idx == 0 { 4.0 } else { 0.0 };
            let right_r = if idx + 1 == len { 4.0 } else { 0.0 };
            (w, color, [left_r, right_r, right_r, left_r])
        })
        .collect()
}

fn keep_alive_btn<'a>(
    model_name: &str,
    label: &'static str,
    val: serde_json::Value,
    highlighted: bool,
    disabled: bool,
) -> Element<'a, Message> {
    let btn = if highlighted {
        button::suggested(label)
    } else {
        button::standard(label)
    };
    if disabled {
        btn.into()
    } else {
        btn.on_press(Message::SetKeepAlive(model_name.to_string(), val))
            .into()
    }
}

fn preset_chip<'a>(label: &'static str, selected: bool, msg: Message) -> Element<'a, Message> {
    if selected {
        button::suggested(label).on_press(msg).into()
    } else {
        button::text(label).on_press(msg).into()
    }
}

fn solid_card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(12)
        .width(Length::Fill)
        .class(theme::Container::Custom(Box::new(|theme| {
            let cosmic = theme.cosmic();
            let palette = cosmic.background(false);
            let mut bg: Color = palette.component.base.into();
            bg.a = 1.0;
            let mut border_color: Color = palette.component.divider.into();
            border_color.a = 1.0;

            cosmic::iced::widget::container::Style {
                text_color: Some(palette.component.on.into()),
                background: Some(bg.into()),
                border: Border {
                    radius: cosmic.corner_radii.radius_s.into(),
                    width: 1.0,
                    color: border_color,
                },
                shadow: Shadow::default(),
                icon_color: Some(palette.component.on.into()),
                snap: true,
            }
        })))
        .into()
}

fn kv_row<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    row([])
        .align_y(Alignment::Center)
        .spacing(8)
        .push(text::caption(label))
        .push(
            container(text::body(value))
                .width(Length::Fill)
                .align_x(Horizontal::Right),
        )
        .into()
}

async fn copy_to_wayland_clipboard_and_wait(text: String) {
    use std::process::Stdio;
    let _ = tokio::process::Command::new("wl-copy")
        .arg("--")
        .arg(&text)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
}

impl Application for OllamaApplet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let app = Self {
            core,
            popup: None,
            view_mode: PopupView::Overview,
            snapshot: OllamaSnapshot::default(),
            telemetry: TelemetryState::default(),
            now: Utc::now(),
            refreshing: true,
            busy_models: HashSet::new(),
            status_banner: None,
            last_error: None,
            endpoint_copied: false,
            endpoint_copied_seq: 0,
        };
        let initial_fetch = Task::perform(ollama::fetch_snapshot(), |snap| {
            Action::App(Message::SnapshotLoaded(snap))
        });
        let init_telemetry = Task::perform(telemetry::init_telemetry(), |t| {
            Action::App(Message::TelemetryInitialized(t))
        });
        (app, Task::batch([initial_fetch, init_telemetry]))
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let poll_sub =
            cosmic::iced::time::every(Duration::from_secs(5)).map(|_| Message::PollPsOnly);
        let journal_sub = telemetry::journal_subscription().map(Message::TelemetryStreamEvent);

        let need_countdown = self.popup.is_some()
            && matches!(self.view_mode, PopupView::Overview)
            && self
                .snapshot
                .running
                .iter()
                .any(|m| m.expires_at.is_some());

        if need_countdown {
            let tick_sub =
                cosmic::iced::time::every(Duration::from_secs(1)).map(|_| Message::TickNow);
            Subscription::batch([poll_sub, journal_sub, tick_sub])
        } else {
            Subscription::batch([poll_sub, journal_sub])
        }
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    self.view_mode = PopupView::Overview;
                    self.status_banner = None;
                    destroy_popup(p)
                } else {
                    let Some(main_id) = self.core.main_window_id() else {
                        return Task::none();
                    };
                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    self.view_mode = PopupView::Overview;
                    self.status_banner = None;
                    self.last_error = None;
                    self.endpoint_copied = false;
                    self.now = Utc::now();
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        main_id,
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(POPUP_WIDTH)
                        .min_width(POPUP_WIDTH)
                        .min_height(120.0)
                        .max_height(1000.0);

                    let open_task = get_popup(popup_settings);
                    let refresh_task = Task::perform(ollama::fetch_snapshot(), |snap| {
                        Action::App(Message::SnapshotLoaded(snap))
                    });
                    Task::batch([open_task, refresh_task])
                }
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                    self.view_mode = PopupView::Overview;
                    self.status_banner = None;
                }
                Task::none()
            }
            Message::RefreshFull => {
                if self.refreshing {
                    return Task::none();
                }
                self.refreshing = true;
                Task::perform(ollama::fetch_snapshot(), |snap| {
                    Action::App(Message::SnapshotLoaded(snap))
                })
            }
            Message::PollPsOnly => {
                if self.refreshing {
                    return Task::none();
                }
                self.refreshing = true;
                let prev = self.snapshot.clone();
                Task::perform(ollama::fetch_ps_only(prev), |snap| {
                    Action::App(Message::SnapshotLoaded(snap))
                })
            }
            Message::SnapshotLoaded(snapshot) => {
                self.refreshing = false;
                self.now = Utc::now();
                self.snapshot = snapshot;
                Task::none()
            }
            Message::TelemetryInitialized(state) => {
                self.telemetry = state;
                Task::none()
            }
            Message::TelemetryStreamEvent(TelemetryEvent::JournalJsonLine(line)) => {
                match self.telemetry.ingest_json_line(&line) {
                    IngestOutcome::RequestCompleted => {
                        let store_clone = self.telemetry.store.clone();
                        let save_task = Task::perform(
                            async move {
                                let _ = telemetry::save_usage_store(&store_clone).await;
                            },
                            |_| Action::App(Message::UsageStoreSaved),
                        );
                        if !self.refreshing {
                            self.refreshing = true;
                            let prev = self.snapshot.clone();
                            let ps_task = Task::perform(ollama::fetch_ps_only(prev), |snap| {
                                Action::App(Message::SnapshotLoaded(snap))
                            });
                            Task::batch([save_task, ps_task])
                        } else {
                            save_task
                        }
                    }
                    IngestOutcome::ModelActivityDetected => {
                        if !self.refreshing {
                            self.refreshing = true;
                            let prev = self.snapshot.clone();
                            Task::perform(ollama::fetch_ps_only(prev), |snap| {
                                Action::App(Message::SnapshotLoaded(snap))
                            })
                        } else {
                            Task::none()
                        }
                    }
                    IngestOutcome::LiveProgressUpdated | IngestOutcome::Ignored => Task::none(),
                }
            }
            Message::UsageStoreSaved => Task::none(),
            Message::OpenHistory => {
                self.last_error = None;
                self.status_banner = None;
                self.view_mode = PopupView::History {
                    confirm_clear: false,
                };
                Task::none()
            }
            Message::RequestClearHistory => {
                self.view_mode = PopupView::History {
                    confirm_clear: true,
                };
                Task::none()
            }
            Message::ConfirmClearHistory => {
                let cursor = self.telemetry.store.last_cursor.clone();
                Task::perform(telemetry::clear_usage_history(cursor), |store| {
                    Action::App(Message::HistoryCleared(store))
                })
            }
            Message::HistoryCleared(store) => {
                self.telemetry.store = store;
                self.telemetry.last_input_tokens = 0;
                self.telemetry.last_output_tokens = 0;
                self.telemetry.last_tok_per_sec = None;
                self.view_mode = PopupView::History {
                    confirm_clear: false,
                };
                Task::none()
            }
            Message::BackToOverview => {
                self.last_error = None;
                self.view_mode = PopupView::Overview;
                Task::none()
            }
            Message::TickNow => {
                self.now = Utc::now();
                Task::none()
            }
            Message::UnloadModel(name) => {
                if self.busy_models.contains(&name) {
                    return Task::none();
                }
                self.last_error = None;
                self.busy_models.insert(name.clone());
                Task::perform(ollama::unload_model(name), |res| {
                    Action::App(Message::ActionCompleted(res))
                })
            }
            Message::LoadModel(name) => {
                if self.busy_models.contains(&name) {
                    return Task::none();
                }
                self.last_error = None;
                self.busy_models.insert(name.clone());
                Task::perform(ollama::load_model(name), |res| {
                    Action::App(Message::ActionCompleted(res))
                })
            }
            Message::SetKeepAlive(name, keep_alive) => {
                if self.busy_models.contains(&name) {
                    return Task::none();
                }
                self.last_error = None;
                self.busy_models.insert(name.clone());
                Task::perform(ollama::set_keep_alive(name, keep_alive), |res| {
                    Action::App(Message::ActionCompleted(res))
                })
            }
            Message::ActionCompleted(result) => {
                match result {
                    Ok(model) => {
                        self.busy_models.remove(&model);
                        self.last_error = None;
                    }
                    Err(err) => {
                        self.busy_models.clear();
                        self.last_error = Some(err);
                    }
                }
                self.refreshing = true;
                Task::perform(ollama::fetch_snapshot(), |snap| {
                    Action::App(Message::SnapshotLoaded(snap))
                })
            }
            Message::OpenConfigure(model_name) => {
                self.last_error = None;
                self.status_banner = None;
                if let Some(running) = self
                    .snapshot
                    .running
                    .iter()
                    .find(|m| m.name == model_name)
                {
                    let draft = ConfigDraft::from_params(
                        running.name.clone(),
                        running.max_context_length,
                        true,
                        false,
                        &running.params,
                    );
                    self.view_mode = PopupView::Configure(draft);
                    Task::none()
                } else {
                    let max_ctx = self
                        .snapshot
                        .installed
                        .iter()
                        .find(|m| m.name == model_name)
                        .and_then(|m| m.context_length);
                    let draft = ConfigDraft::from_params(
                        model_name.clone(),
                        max_ctx,
                        false,
                        true,
                        &ModelParams::default(),
                    );
                    self.view_mode = PopupView::Configure(draft);
                    Task::perform(ollama::fetch_model_params(model_name), |res| {
                        Action::App(Message::ModelParamsLoaded(res))
                    })
                }
            }
            Message::ModelParamsLoaded(res) => {
                match res {
                    Ok((model_name, params)) => {
                        if let PopupView::Configure(draft) = &mut self.view_mode {
                            if draft.source_model == model_name {
                                draft.apply_fetched(params);
                            }
                        }
                    }
                    Err(err) => {
                        if let PopupView::Configure(draft) = &mut self.view_mode {
                            draft.loading_params = false;
                        }
                        self.last_error = Some(err);
                    }
                }
                Task::none()
            }
            Message::ConfigTempChanged(v) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.temperature = v;
                }
                Task::none()
            }
            Message::ConfigTopPChanged(v) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.top_p = v;
                }
                Task::none()
            }
            Message::ConfigTopKChanged(v) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.top_k = v;
                }
                Task::none()
            }
            Message::ConfigRepeatChanged(v) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.repeat_penalty = v;
                }
                Task::none()
            }
            Message::ConfigNumCtxChanged(s) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.num_ctx_input = s;
                }
                Task::none()
            }
            Message::ConfigNumPredictChanged(s) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.num_predict_input = s;
                }
                Task::none()
            }
            Message::ConfigSaveAsChanged(s) => {
                if let PopupView::Configure(draft) = &mut self.view_mode {
                    draft.save_as_tag = s;
                }
                Task::none()
            }
            Message::SaveConfiguration => {
                let PopupView::Configure(draft) = &mut self.view_mode else {
                    return Task::none();
                };
                if draft.saving {
                    return Task::none();
                }
                let params = match draft.build_model_params() {
                    Ok(p) => p,
                    Err(validation_err) => {
                        self.last_error = Some(validation_err);
                        return Task::none();
                    }
                };
                self.last_error = None;
                draft.saving = true;
                let source = draft.source_model.clone();
                let target = draft.target_model_name();
                let was_running = draft.was_running;

                Task::perform(
                    ollama::save_model_params(source, target, params, was_running),
                    |res| Action::App(Message::ConfigurationSaved(res)),
                )
            }
            Message::ConfigurationSaved(res) => match res {
                Ok(saved_name) => {
                    self.last_error = None;
                    self.status_banner = Some(format!("Saved parameters to {saved_name}"));
                    self.view_mode = PopupView::Overview;
                    self.refreshing = true;
                    Task::perform(ollama::fetch_snapshot(), |snap| {
                        Action::App(Message::SnapshotLoaded(snap))
                    })
                }
                Err(err) => {
                    if let PopupView::Configure(draft) = &mut self.view_mode {
                        draft.saving = false;
                    }
                    self.last_error = Some(err);
                    Task::none()
                }
            },
            Message::CopyEndpoint => {
                self.endpoint_copied = true;
                self.endpoint_copied_seq = self.endpoint_copied_seq.wrapping_add(1);
                let seq = self.endpoint_copied_seq;
                let endpoint = self.snapshot.host.clone();
                let iced_clip = cosmic::iced::clipboard::write(endpoint.clone());
                let wl_and_reset = Task::perform(
                    copy_to_wayland_clipboard_and_wait(endpoint),
                    move |_| Action::App(Message::ClearCopiedFeedback(seq)),
                );
                Task::batch([iced_clip, wl_and_reset])
            }
            Message::ClearCopiedFeedback(seq) => {
                if self.endpoint_copied_seq == seq {
                    self.endpoint_copied = false;
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let suggested = self.core.applet.suggested_size(true);
        let (major_pad, minor_pad) = self.core.applet.suggested_padding(true);

        if self.core.applet.is_horizontal() {
            let content = row([])
                .align_y(Alignment::Center)
                .spacing(6)
                .push(self.symbolic_icon(suggested.0))
                .push(self.core.applet.text(self.panel_label()));

            let btn = button::custom(
                widget::layer_container(content)
                    .center_y(Length::Fixed(f32::from(suggested.1 + 2 * minor_pad))),
            )
            .padding([0, major_pad])
            .class(theme::Button::AppletIcon)
            .on_press_down(Message::TogglePopup);

            self.core.applet.autosize_window(btn).into()
        } else {
            let handle = icon::from_svg_bytes(ICON_SVG).symbolic(true);
            let btn = self
                .core
                .applet
                .icon_button_from_handle(handle)
                .on_press_down(Message::TogglePopup);

            self.core.applet.autosize_window(btn).into()
        }
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let inner = match &self.view_mode {
            PopupView::Overview => self.render_overview(),
            PopupView::History { confirm_clear } => self.render_history(*confirm_clear),
            PopupView::Configure(draft) => self.render_configure(draft),
        };
        self.solid_popup_container(inner)
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_horizontal_segments_with_cached() {
        let day = DailyTokens {
            input_tokens: 11_825,
            cached_input_tokens: 1_538,
            output_tokens: 6_057,
            requests: 10,
        };
        let segs = build_horizontal_segments(&day, 27_002, 224.0);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].2, [4.0, 0.0, 0.0, 4.0]);
        assert_eq!(segs[1].2, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(segs[2].2, [0.0, 4.0, 4.0, 0.0]);
        let sum_w: f32 = segs.iter().map(|(w, _, _)| *w).sum();
        assert!((sum_w - 148.0).abs() <= 1.0);
        assert!(segs.iter().all(|(w, _, _)| *w >= 3.0));
    }
}
