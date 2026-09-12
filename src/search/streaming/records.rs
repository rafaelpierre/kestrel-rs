//! Frame boundaries are irrelevant: adapters yield only closed HTML cards or JSON items.
use super::super::*;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, states::RawKind,
};
use std::cell::RefCell;

const MAX_PASSES: usize = 256;
const MAX_DEPTH: usize = 128;

pub(super) struct Records {
    engine: Engine,
    text: String,
    json: Option<JsonRecords>,
    html: Option<HtmlWorker>,
    passes: usize,
}
impl Records {
    pub fn new(engine: Engine) -> Self {
        let json = matches!(
            engine,
            Engine::Dogpile | Engine::Yep | Engine::Qwant | Engine::Swisscows
        )
        .then(JsonRecords::default);
        let html = json.is_none().then(|| HtmlWorker::new(engine));
        Self {
            engine,
            text: String::new(),
            json,
            html,
            passes: 0,
        }
    }
    pub async fn push(&mut self, text: &str) -> Result<Option<Vec<SearchResult>>, KestrelError> {
        if let Some(html) = &self.html {
            return html.push(text).await;
        }
        self.text.push_str(text);
        if self.passes >= MAX_PASSES {
            return Ok(None);
        }
        let snapshot = self
            .json
            .as_mut()
            .and_then(|json| json.push(&self.text, self.engine));
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        self.passes += 1;
        if classify_challenge(self.engine, &snapshot) == Challenge::Detected {
            return Err(KestrelError::Search(format!(
                "{} returned a bot challenge",
                self.engine
            )));
        }
        Ok(parse_provider_response(self.engine, &snapshot).ok())
    }
}

#[derive(Default)]
struct JsonRecords {
    offset: usize,
    string: bool,
    escaped: bool,
    disabled: bool,
    stack: Vec<(u8, bool)>,
    probes: usize,
}

impl JsonRecords {
    fn push(&mut self, text: &str, engine: Engine) -> Option<String> {
        if self.disabled {
            return None;
        }
        let mut snapshot = None;
        for (i, byte) in text.bytes().enumerate().skip(self.offset) {
            if self.string {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.string = false;
                }
                continue;
            }
            match byte {
                b'"' => self.string = true,
                b'{' | b'[' => {
                    if self.stack.len() >= MAX_DEPTH {
                        self.disabled = true;
                        return None;
                    }
                    if byte == b'[' {
                        self.probes += 1;
                    }
                    if self.probes > MAX_PASSES {
                        self.disabled = true;
                        return None;
                    }
                    let target = byte == b'[' && target_array(engine, &text[..=i], &self.stack);
                    self.stack.push((byte, target));
                }
                b'}' | b']' => {
                    let Some((open, _)) = self.stack.pop() else {
                        self.disabled = true;
                        return None;
                    };
                    if (open == b'{' && byte != b'}') || (open == b'[' && byte != b']') {
                        self.disabled = true;
                        return None;
                    }
                    if byte == b'}' && self.stack.last().is_some_and(|(_, target)| *target) {
                        snapshot = Some((i, self.stack.clone()));
                    }
                }
                _ => {}
            }
        }
        self.offset = text.len();
        snapshot.map(|(end, stack)| {
            let mut complete = text[..=end].to_owned();
            close_json(&mut complete, &stack);
            complete
        })
    }
}

fn close_json(text: &mut String, stack: &[(u8, bool)]) {
    for (kind, _) in stack.iter().rev() {
        text.push(if *kind == b'{' { '}' } else { ']' });
    }
}

fn target_array(engine: Engine, prefix: &str, stack: &[(u8, bool)]) -> bool {
    let mut probe = prefix.to_owned();
    probe.push_str("\"__kestrel_current_array__\"]");
    close_json(&mut probe, stack);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&probe) else {
        return false;
    };
    let marker = serde_json::json!(["__kestrel_current_array__"]);
    match engine {
        Engine::Dogpile => value.get("results") == Some(&marker),
        Engine::Swisscows => value.get("items") == Some(&marker),
        Engine::Yep => {
            value.get(0).and_then(|v| v.as_str()) == Some("Ok")
                && value.pointer("/1/results") == Some(&marker)
        }
        Engine::Qwant => {
            value.get("status").and_then(|v| v.as_str()) == Some("success")
                && value
                    .pointer("/data/result/items/mainline")
                    .and_then(|v| v.as_array())
                    .is_some_and(|rows| {
                        rows.iter().any(|row| {
                            row.get("type").and_then(|v| v.as_str()) == Some("web")
                                && row.get("items") == Some(&marker)
                        })
                    })
        }
        _ => false,
    }
}

struct HtmlWorker {
    sender: tokio::sync::mpsc::Sender<HtmlJob>,
}
type HtmlJob = (
    String,
    tokio::sync::oneshot::Sender<Result<Option<Vec<SearchResult>>, KestrelError>>,
);

impl HtmlWorker {
    fn new(engine: Engine) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<HtmlJob>(1);
        tokio::task::spawn_blocking(move || {
            // The tokenizer is thread-local; only owned strings and results cross the channel.
            let tokenizer = Tokenizer::new(
                HtmlSink {
                    engine,
                    state: RefCell::new(HtmlState::default()),
                },
                Default::default(),
            );
            let input = BufferQueue::default();
            let mut results = Vec::new();
            while let Some((text, reply)) = receiver.blocking_recv() {
                input.push_back(text.into());
                let _ = tokenizer.feed(&input);
                let mut state = tokenizer.sink.state.borrow_mut();
                let outcome = if state.challenge {
                    Err(KestrelError::Search(format!(
                        "{engine} returned a bot challenge"
                    )))
                } else if state.disabled {
                    Ok(None)
                } else {
                    let old = results.len();
                    for card in state.completed.drain(..) {
                        let card = if engine == Engine::Mojeek {
                            format!("<ul class='results-standard'>{card}</ul>")
                        } else {
                            card
                        };
                        if let Ok(batch) = parse_provider_response(engine, &card) {
                            results.extend(batch);
                        }
                    }
                    Ok((results.len() != old).then(|| results.clone()))
                };
                if reply.send(outcome).is_err() {
                    break;
                }
            }
            // No EOF recovery: only complete cards are published. Dropping the sender stops the worker.
        });
        Self { sender }
    }
    async fn push(&self, text: &str) -> Result<Option<Vec<SearchResult>>, KestrelError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send((text.to_owned(), sender))
            .await
            .map_err(|_| KestrelError::Search("HTML stream parser stopped".into()))?;
        receiver
            .await
            .map_err(|_| KestrelError::Search("HTML stream parser stopped".into()))?
    }
}

struct HtmlSink {
    engine: Engine,
    state: RefCell<HtmlState>,
}
#[derive(Default)]
struct HtmlState {
    card: String,
    stack: Vec<String>,
    card_depth: Option<usize>,
    completed: Vec<String>,
    disabled: bool,
    challenge: bool,
    title: String,
    in_title: bool,
    challenge_text: String,
    in_challenge: bool,
    opaque_depth: usize,
}

impl TokenSink for HtmlSink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut state = self.state.borrow_mut();
        if state.disabled {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind == TagKind::StartTag {
                    let attribute = |key: &str| {
                        tag.attrs
                            .iter()
                            .find(|a| a.name.local.as_ref() == key)
                            .map(|a| a.value.as_ref())
                            .unwrap_or("")
                    };
                    let class = |value: &str| {
                        attribute("class")
                            .split_ascii_whitespace()
                            .any(|c| c == value)
                    };
                    let id = attribute("id");
                    if matches!(
                        id,
                        "b_captcha" | "captcha" | "challenge-form" | "cf-challenge-running"
                    ) || class("g-recaptcha")
                        || class("anomaly-modal")
                        || (name == "form"
                            && (attribute("action").contains("captcha")
                                || attribute("action").contains("anomaly.js")))
                    {
                        state.challenge = true;
                    }
                    if name == "title" {
                        state.in_title = true;
                        state.title.clear();
                    }
                    if class("captcha-wrap") {
                        state.in_challenge = true;
                    }
                    if matches!(name, "template" | "svg" | "math") {
                        state.opaque_depth += 1;
                    }
                    if name == "plaintext" {
                        state.disabled = true;
                        return TokenSinkResult::Plaintext;
                    }
                    let card = state.opaque_depth == 0
                        && match self.engine {
                            Engine::Bing => name == "li" && class("b_algo"),
                            Engine::Duckduckgo => {
                                name == "div"
                                    && [
                                        "result",
                                        "results_links",
                                        "results_links_deep",
                                        "web-result",
                                    ]
                                    .iter()
                                    .all(|c| class(c))
                            }
                            Engine::Yahoo => name == "div" && class("dd") && class("algo"),
                            Engine::Ecosia => class("result") || class("web-result"),
                            Engine::Mojeek => {
                                name == "li"
                                    && state
                                        .stack
                                        .last()
                                        .is_some_and(|n| n == "ul:results-standard")
                            }
                            _ => false,
                        };
                    if card && state.card_depth.is_none() {
                        state.card_depth = Some(state.stack.len());
                    }
                    if state.card_depth.is_some() {
                        state.card.push('<');
                        state.card.push_str(name);
                        for attr in &tag.attrs {
                            state.card.push(' ');
                            state.card.push_str(attr.name.local.as_ref());
                            state.card.push_str("=\"");
                            state
                                .card
                                .push_str(&html_escape::encode_double_quoted_attribute(
                                    &attr.value,
                                ));
                            state.card.push('"');
                        }
                        state.card.push('>');
                    }
                    let void = matches!(
                        name,
                        "area"
                            | "base"
                            | "br"
                            | "col"
                            | "embed"
                            | "hr"
                            | "img"
                            | "input"
                            | "link"
                            | "meta"
                            | "param"
                            | "source"
                            | "track"
                            | "wbr"
                    );
                    if !void {
                        if state.stack.len() >= MAX_DEPTH {
                            state.disabled = true;
                            return TokenSinkResult::Continue;
                        }
                        state
                            .stack
                            .push(if name == "ul" && class("results-standard") {
                                "ul:results-standard".into()
                            } else {
                                name.to_owned()
                            });
                    }
                    return match name {
                        "script" => TokenSinkResult::RawData(RawKind::ScriptData),
                        "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                            TokenSinkResult::RawData(RawKind::Rawtext)
                        }
                        "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
                        _ => TokenSinkResult::Continue,
                    };
                } else {
                    if name == "title" {
                        state.in_title = false;
                    }
                    if matches!(name, "template" | "svg" | "math") {
                        state.opaque_depth = state.opaque_depth.saturating_sub(1);
                    }
                    let position = state
                        .stack
                        .iter()
                        .rposition(|n| n.split(':').next() == Some(name));
                    if let Some(position) = position {
                        if let Some(root) = state.card_depth {
                            // HTML permits these optional end tags. Other mismatches fall back to EOF.
                            if state.stack[position + 1..].iter().any(|n| {
                                !matches!(
                                    n.as_str(),
                                    "p" | "li"
                                        | "dt"
                                        | "dd"
                                        | "rt"
                                        | "rp"
                                        | "option"
                                        | "optgroup"
                                        | "thead"
                                        | "tbody"
                                        | "tfoot"
                                        | "tr"
                                        | "td"
                                        | "th"
                                )
                            }) {
                                state.disabled = true;
                                return TokenSinkResult::Continue;
                            }
                            state.card.push_str("</");
                            state.card.push_str(name);
                            state.card.push('>');
                            if position == root {
                                let card = std::mem::take(&mut state.card);
                                state.completed.push(card);
                                state.card_depth = None;
                            }
                        }
                        state.stack.truncate(position);
                    }
                }
            }
            Token::CharacterTokens(chars) => {
                if state.in_title {
                    state.title.push_str(&chars);
                }
                if state.in_challenge {
                    state.challenge_text.push_str(&chars);
                }
                if state.card_depth.is_some() {
                    state.card.push_str(&html_escape::encode_text(&chars));
                }
                if self.engine == Engine::Ecosia && state.title.contains("Firewall") {
                    state.challenge = true;
                }
                if self.engine == Engine::Mojeek
                    && state.title.trim().eq_ignore_ascii_case("captcha")
                    && state
                        .challenge_text
                        .to_ascii_lowercase()
                        .contains("javascript is required to complete this challenge.")
                {
                    state.challenge = true;
                }
            }
            Token::NullCharacterToken => {
                state.disabled = true;
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Vec<(Engine, &'static str)> {
        vec![
            (
                Engine::Bing,
                r#"<ol><li class="b_algo"><h2><a href="https://example.org/">Café &amp; Rust</a></h2><div class="b_caption"><p>Complete snippet.</p></div></li></ol>"#,
            ),
            (
                Engine::Yahoo,
                r#"<div class="dd algo"><div class="compTitle"><h3><a href="https://example.org/">Café</a></h3></div><div class="compText"><p>Complete snippet.</p></div></div>"#,
            ),
            (
                Engine::Duckduckgo,
                r#"<div class="result results_links results_links_deep web-result"><h2 class="result__title"><a class="result__a" href="https://example.org/">Café</a></h2><a class="result__snippet">Complete snippet.</a></div>"#,
            ),
            (
                Engine::Ecosia,
                include_str!("../../../tests/fixtures/providers/ecosia.html"),
            ),
            (
                Engine::Mojeek,
                include_str!("../../../tests/fixtures/providers/mojeek.html"),
            ),
            (
                Engine::Dogpile,
                include_str!("../../../tests/fixtures/providers/dogpile.json"),
            ),
            (
                Engine::Swisscows,
                include_str!("../../../tests/fixtures/providers/swisscows.json"),
            ),
            (
                Engine::Yep,
                include_str!("../../../tests/fixtures/providers/yep.json"),
            ),
            (
                Engine::Qwant,
                include_str!("../../../tests/fixtures/providers/qwant.json"),
            ),
        ]
    }

    #[tokio::test]
    async fn all_providers_emit_the_same_results_as_their_full_parser() {
        for (engine, text) in fixtures() {
            for split in [1, 7, text.len()] {
                let mut parser = Records::new(engine);
                let mut latest = None;
                // Split on character boundaries here; byte/charset splits are transport tests.
                let chars: Vec<_> = text.chars().collect();
                for chunk in chars.chunks(split) {
                    if let Some(results) = parser
                        .push(&chunk.iter().collect::<String>())
                        .await
                        .unwrap()
                    {
                        latest = Some(results);
                    }
                }
                assert_eq!(
                    latest.unwrap_or_default(),
                    parse_provider_response(engine, text).unwrap(),
                    "{engine}, chunks {split}"
                );
            }
        }
    }

    #[tokio::test]
    async fn unfinished_cards_and_script_comment_lookalikes_are_not_records() {
        let fake = r#"<li class="b_algo"><h2><a href="https://example.org/">fake</a></h2></li>"#;
        for text in [
            format!("<!-- {fake} -->"),
            format!("<script>{fake}</script>"),
            fake.trim_end_matches("</li>").to_owned(),
        ] {
            assert!(
                Records::new(Engine::Bing)
                    .push(&text)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        let mut parser = Records::new(Engine::Bing);
        assert!(
            parser
                .push(fake.trim_end_matches("</li>"))
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(parser.push("</li>").await.unwrap().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn nested_json_fields_do_not_complete_the_enclosing_result() {
        let mut parser = Records::new(Engine::Dogpile);
        let prefix = r#"{"results":[{"title":"quoted \\\" title","clickUrl":"https://example.org/","nested":[{"title":"x"}]"#;
        assert!(parser.push(prefix).await.unwrap().is_none());
        let result = parser
            .push(r#", "description":"complete"}"#)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result[0].snippet, "complete");
    }

    #[tokio::test]
    async fn known_challenges_are_rejected_before_publication() {
        let (engine, card) = fixtures().remove(0);
        assert!(
            Records::new(engine)
                .push(&format!("<div id='b_captcha'></div>{card}"))
                .await
                .is_err()
        );
        let mut qwant = Records::new(Engine::Qwant);
        assert!(qwant.push(r#"{"status":"error","data":{"result":{"items":{"mainline":[{"type":"web","items":[{"title":"x","url":"https://example.org"}]}]}}}}"#).await.unwrap().is_none());
    }
}
