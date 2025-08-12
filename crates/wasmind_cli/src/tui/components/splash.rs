use crate::config::ParsedTuiConfig;
use crate::tui::model::TuiMessage;
use crate::tui::utils::{center, center_horizontal};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::Paragraph;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, Props, State,
    command::{Cmd, CmdResult},
    ratatui::layout::Rect,
};
use wasmind::actors::MessageEnvelope;

use super::textarea::LLMTextAreaComponent;

const SPLASH_TITLE: &str = r#"▗▖ ▗▖▗▄▄▄▖▗▖  ▗▖▗▄▄▄▖
▐▌ ▐▌  █  ▐▌  ▐▌▐▌   
▐▛▀▜▌  █  ▐▌  ▐▌▐▛▀▀▘
▐▌ ▐▌▗▄█▄▖ ▝▚▞▘ ▐▙▄▄▄"#;

fn generate_tag_text() -> &'static str {
    if rand::random::<bool>() {
        "build anything"
    } else {
        "build everything"
    }
}

#[derive(MockComponent)]
pub struct SplashComponent {
    component: Splash,
}

impl SplashComponent {
    pub fn new(config: ParsedTuiConfig) -> Self {
        Self {
            component: Splash {
                state: State::None,
                props: Props::default(),
                textarea_component: LLMTextAreaComponent::new(config.clone()),
                tag_text: generate_tag_text(),
            },
        }
    }
}

struct Splash {
    props: Props,
    state: State,
    textarea_component: LLMTextAreaComponent,
    tag_text: &'static str,
}

impl MockComponent for Splash {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        if self.props.get_or(Attribute::Display, AttrValue::Flag(true)) == AttrValue::Flag(true) {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(frame.area());

            // Render the splash top
            let paragraph = Paragraph::new(SPLASH_TITLE);
            let width = paragraph.line_width() as u16;
            let height = paragraph.line_count(width) as u16;
            let title_area_top = center(
                layout[0],
                Constraint::Length(width),
                Constraint::Length(height),
            );
            frame.render_widget(paragraph, title_area_top);

            // Render the textarea
            let width = area.width.min(100);
            let mut textarea_area = center_horizontal(layout[1], width);
            let height = self.textarea_component.get_height(textarea_area) as u16;
            textarea_area.height = height;
            self.textarea_component.view(frame, textarea_area);

            // Bottom text
            let mut bottom_text_area = layout[1];
            bottom_text_area.y += textarea_area.height + 5;
            let centered_bottom_text_area =
                center_horizontal(bottom_text_area, self.tag_text.chars().count() as u16);
            let paragraph = Paragraph::new(self.tag_text);
            frame.render_widget(paragraph, centered_bottom_text_area);
        }
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.props.get(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    fn state(&self) -> State {
        self.state.clone()
    }

    fn perform(&mut self, _cmd: Cmd) -> CmdResult {
        unreachable!()
    }
}

impl Component<TuiMessage, MessageEnvelope> for SplashComponent {
    fn on(&mut self, ev: Event<MessageEnvelope>) -> Option<TuiMessage> {
        self.component.textarea_component.on(ev)
    }
}
