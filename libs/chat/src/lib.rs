pub mod api;
pub mod dialogue;
pub mod icontray;
pub mod ui;

use std::convert::TryInto;
use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub use api::*;
pub use blitstr2::GlyphStyle;
pub use enumset::EnumSet;
use gam::MenuItem;
use num_traits::FromPrimitive;
pub use ui::BUSY_ANIMATION_RATE_MS;
use ui::VisualProperties;
use ux_api::minigfx::*;
use xous::{CID, Error, SID, msg_scalar_unpack};
use xous_ipc::Buffer;

/// The horizontal span a centered system row (a date separator or the
/// message-request instruction bar; ISSUES r6 rows 21/22) is centered
/// within: the full canvas width less one margin per side, wider than
/// an ordinary bubble's 80% column so a multi-line notice has room.
fn center_span(vp: &VisualProperties) -> (isize, isize) {
    (vp.margin.x, vp.total_screensize.x - vp.margin.x)
}

/// The `TextBounds` for a centered system row, using the
/// `CenteredTop`/`CenteredBot` variants the wider ux-api framework
/// already implements (services/gam modals, apps/vault) for exactly
/// this: the gfx handler measures the composed text and centers it
/// horizontally within the given rectangle
/// (libs/ux-api/src/minigfx/handlers.rs). libs/chat had not used
/// either variant before this change; every other row here still
/// anchors via the `Growable*` family. `topdown` and `anchor_y` follow
/// the same convention `bubble`/`header_bubble` use: topdown anchors
/// the top edge and grows down, otherwise the bottom edge is anchored
/// and growth is up. The height bound is generous (a full screen's
/// worth) because these rows are always short; it never binds.
fn centered_bounds(vp: &VisualProperties, topdown: bool, anchor_y: isize) -> TextBounds {
    let (x0, x1) = center_span(vp);
    if topdown {
        TextBounds::CenteredTop(Rectangle::new(
            Point::new(x0, anchor_y),
            Point::new(x1, anchor_y + vp.layout_screensize.y),
        ))
    } else {
        TextBounds::CenteredBot(Rectangle::new(
            Point::new(x0, anchor_y - vp.layout_screensize.y),
            Point::new(x1, anchor_y),
        ))
    }
}

/// Create a TextView with the default properties common to all text bubbles.
/// A centered post (`post.center()`, ISSUES r6 rows 21/22) is rendered
/// borderless and box-free regardless of `vp.bubbles`, matching the
/// non-bubble surfaces' own rule that a border appears only as the
/// selection cursor.
pub(crate) fn default_textview(
    post: &crate::dialogue::post::Post,
    hilite: bool,
    vp: &VisualProperties,
) -> TextView {
    use std::fmt::Write;
    let bounds_hint = if post.center() {
        // The real x-position is computed at draw time from anchor_y
        // (see `bubble`); this placeholder is only for the
        // insert-time and layout-measure bounds_compute_textview
        // calls, which need the same width to get a consistent
        // height, but do not care about the y position.
        centered_bounds(vp, true, vp.status_height as isize + vp.margin.y)
    } else {
        TextBounds::GrowableFromBl(Point::new(vp.margin.x, vp.layout_screensize.y), vp.bubble_width)
    };
    let mut bubble_tv = TextView::new(vp.canvas, bounds_hint);
    // A centered row is treated as a non-bubble row for framing
    // purposes (bordered only when selected), whatever vp.bubbles is.
    let bordered = vp.bubbles && !post.center();
    if bordered {
        bubble_tv.border_width = if hilite { 3 } else { 1 };
    } else {
        // plain rows carry no frame; a border appears only as the selection cursor
        bubble_tv.border_width = 2;
    }
    bubble_tv.clip_rect =
        Some(Rectangle::new(Point::new(0, vp.status_height as isize + vp.margin.y), vp.layout_screensize));
    bubble_tv.draw_border = bordered || hilite;
    bubble_tv.clear_area = true;
    bubble_tv.rounded_border = if bordered { Some(vp.bubble_radius) } else { None };
    bubble_tv.style = vp.style;
    bubble_tv.margin = vp.bubble_margin;
    bubble_tv.ellipsis = false;
    bubble_tv.insertion = None;
    write!(bubble_tv.text, "{}", post.text()).expect("couldn't write history text to TextView");
    bubble_tv
}

/// The chrome line above a bubble: Bold, no border, no rounding,
/// one bubble-margin of x-indent. Returns None unless this surface
/// draws bubbles, because a borderless full-width row (boot log,
/// overview) has no border for the header to sit outside of, and the
/// anti-spoof property is the border
/// (ignore/notes-r5-author-identity.md section 4): body glyphs are
/// clipped inside the bubble border, the header is drawn outside it,
/// so no body string can render where the header renders.
pub(crate) fn header_textview(
    post: &crate::dialogue::post::Post,
    vp: &VisualProperties,
) -> Option<TextView> {
    use std::fmt::Write;
    if !crate::ui::header_wanted(vp.bubbles, post.header()) {
        return None;
    }
    let header = post.header()?;
    let mut tv = TextView::new(
        vp.canvas,
        TextBounds::GrowableFromBl(Point::new(vp.margin.x, vp.layout_screensize.y), vp.bubble_width),
    );
    tv.style = GlyphStyle::Bold;
    tv.draw_border = false;
    tv.border_width = 0;
    tv.rounded_border = None;
    tv.clear_area = true;
    tv.margin = Point::new(vp.bubble_margin.x, 0);
    tv.ellipsis = true;
    tv.insertion = None;
    tv.clip_rect =
        Some(Rectangle::new(Point::new(0, vp.status_height as isize + vp.margin.y), vp.layout_screensize));
    write!(tv.text, "{}", header).ok();
    Some(tv)
}

/// Position a header TextView the way `bubble` positions the body:
/// the anchor corner follows the layout direction and the author's
/// Right flag. In practice only the left cases run today (only
/// incoming rows carry a header), but both sides are written.
pub(crate) fn header_bubble(
    vp: &VisualProperties,
    topdown: bool,
    post: &crate::dialogue::post::Post,
    dialogue: &crate::dialogue::Dialogue,
    anchor_y: isize,
) -> Option<TextView> {
    let mut tv = header_textview(post, vp)?;
    let mut align_right = false;
    let mut anchor_x = vp.margin.x;
    if let Some(author) = dialogue.author(post.author_id()) {
        if author.flag_is(AuthorFlag::Right) {
            align_right = true;
            anchor_x = vp.layout_screensize.x - vp.margin.x;
        }
    }
    let anchor = Point::new(anchor_x, anchor_y);
    let width = vp.bubble_width;
    tv.bounds_hint = match (topdown, align_right) {
        (true, true) => TextBounds::GrowableFromTr(anchor, width),
        (true, false) => TextBounds::GrowableFromTl(anchor, width),
        (false, true) => TextBounds::GrowableFromBr(anchor, width),
        (false, false) => TextBounds::GrowableFromBl(anchor, width),
    };
    Some(tv)
}

/// The chrome line below a bubble: Regular, no border, no rounding,
/// one bubble-margin of x-indent, aligned to the same side as the
/// bubble it belongs to (ISSUES r6 row 20). Mirrors `header_textview`
/// exactly except for style and vertical placement; `GlyphStyle::
/// Regular` rather than `Small` because the 16 px emoji glyph the
/// delivery marks use sets the line height either way (16 px against
/// 15 px Regular; a 12 px Small line would not shrink the row).
/// Returns None on the same conditions as the header: no bubbles, or
/// no footer string.
pub(crate) fn footer_textview(
    post: &crate::dialogue::post::Post,
    vp: &VisualProperties,
) -> Option<TextView> {
    use std::fmt::Write;
    if !crate::ui::footer_wanted(vp.bubbles, post.footer()) {
        return None;
    }
    let footer = post.footer()?;
    let mut tv = TextView::new(
        vp.canvas,
        TextBounds::GrowableFromBl(Point::new(vp.margin.x, vp.layout_screensize.y), vp.bubble_width),
    );
    tv.style = GlyphStyle::Regular;
    tv.draw_border = false;
    tv.border_width = 0;
    tv.rounded_border = None;
    tv.clear_area = true;
    tv.margin = Point::new(vp.bubble_margin.x, 0);
    tv.ellipsis = true;
    tv.insertion = None;
    tv.clip_rect =
        Some(Rectangle::new(Point::new(0, vp.status_height as isize + vp.margin.y), vp.layout_screensize));
    write!(tv.text, "{}", footer).ok();
    Some(tv)
}

/// Position a footer TextView the way `header_bubble` positions the
/// header: same anchor-corner rule (layout direction, author's Right
/// flag), just below the bubble instead of above it.
pub(crate) fn footer_bubble(
    vp: &VisualProperties,
    topdown: bool,
    post: &crate::dialogue::post::Post,
    dialogue: &crate::dialogue::Dialogue,
    anchor_y: isize,
) -> Option<TextView> {
    let mut tv = footer_textview(post, vp)?;
    let mut align_right = false;
    let mut anchor_x = vp.margin.x;
    if let Some(author) = dialogue.author(post.author_id()) {
        if author.flag_is(AuthorFlag::Right) {
            align_right = true;
            anchor_x = vp.layout_screensize.x - vp.margin.x;
        }
    }
    let anchor = Point::new(anchor_x, anchor_y);
    let width = vp.bubble_width;
    tv.bounds_hint = match (topdown, align_right) {
        (true, true) => TextBounds::GrowableFromTr(anchor, width),
        (true, false) => TextBounds::GrowableFromTl(anchor, width),
        (false, true) => TextBounds::GrowableFromBr(anchor, width),
        (false, false) => TextBounds::GrowableFromBl(anchor, width),
    };
    Some(tv)
}

/// Return a TextView bubble representing a Dialogue Post
///
/// # Arguments
///
/// * `vp` - the visual properties to be applied to the textview
/// * `topdown` - direction of the layout
/// * `post` - the post to represent in a TextView bubble
/// * `dialogue` - containing the Post for context info
/// * `hilite` - hilite this Post on the screen (thicker border)
/// * `anchor_y` - the vertical position on screen to draw TextView bubble
fn bubble(
    vp: &VisualProperties,
    topdown: bool,
    post: &crate::dialogue::post::Post,
    dialogue: &crate::dialogue::Dialogue,
    hilite: bool,
    anchor_y: isize,
) -> TextView {
    // create a textview with all of our default properties
    let mut bubble_tv = default_textview(post, hilite, vp);

    // A centered system row (ISSUES r6 rows 21/22) is the third case
    // of the "measure the text, set the origin" machinery Right-align
    // already established below: CenteredTop/CenteredBot measure the
    // composed text and center it, so no author lookup or explicit
    // width math is needed here.
    if post.center() {
        bubble_tv.bounds_hint = centered_bounds(vp, topdown, anchor_y);
        return bubble_tv;
    }

    // set alignment of bubble left/right
    let mut align_right = false;
    let mut anchor_x = vp.margin.x; // default to align left
    if let Some(author) = dialogue.author(post.author_id()) {
        if author.flag_is(AuthorFlag::Right) {
            // align right
            align_right = true;
            anchor_x = vp.layout_screensize.x - vp.margin.x;
        }
    }
    // set the text bounds of the bubble and the growth direction
    let anchor = Point::new(anchor_x, anchor_y);
    let width = vp.bubble_width;
    let text_bounds = match (topdown, align_right) {
        (true, true) => TextBounds::GrowableFromTr(anchor, width),
        (true, false) => TextBounds::GrowableFromTl(anchor, width),
        (false, true) => TextBounds::GrowableFromBr(anchor, width),
        (false, false) => TextBounds::GrowableFromBl(anchor, width),
    };

    bubble_tv.bounds_hint = text_bounds;
    bubble_tv
}

pub struct Chat {
    cid: CID,
}

impl Chat {
    /// Create a new Chat UI
    ///
    /// # Arguments
    ///
    /// * `app_name` - registered with GAM
    /// * `app_menu` - with menu items handled by the Chat App rather than the Chat UI
    /// * `app_cid` - to accept messages from the Chat UI (see below)
    /// * `post_opcode` - to handle a `MemoryMessage` containing a new outbound user Post
    /// * `event_opcode` - to handle `ScalarMessage` representing a UI Event, such as F1 click, Left click,
    ///   Top Post, etc.
    /// * `rawkeys_opcode` - to handle a raw-keystroke.
    pub fn new(
        app_name: &'static str,
        app_menu: &'static str,
        app_cid: Option<CID>,
        opcode_post: Option<usize>,
        opcode_event: Option<usize>,
        opcode_rawkeys: Option<usize>,
    ) -> Self {
        let chat_sid = xous::create_server().unwrap();
        let chat_cid = xous::connect(chat_sid).unwrap();

        let busy_bumper = xous::create_server().unwrap();
        let busy_bumper_cid = xous::connect(busy_bumper).unwrap();

        log::info!("starting idle animation runner");
        let run_busy_animation = Arc::new(AtomicBool::new(false));
        thread::spawn({
            let run_busy_animation = run_busy_animation.clone();
            move || {
                busy_animator(busy_bumper, busy_bumper_cid, chat_cid, run_busy_animation);
            }
        });

        log::info!("Starting chat UI server",);
        thread::spawn({
            move || {
                server(
                    chat_sid,
                    app_name,
                    app_menu,
                    app_cid,
                    opcode_post,
                    opcode_event,
                    opcode_rawkeys,
                    run_busy_animation,
                    busy_bumper_cid,
                );
            }
        });

        Chat { cid: chat_cid }
    }

    /// Return the Chat App CID
    ///
    /// This cid allows the Chat App to contact this Chat UI server
    pub fn cid(&self) -> CID { self.cid }

    /// Create an offline/read-only Chat UI over and existing Dialogue in pddb
    ///
    /// # Arguments
    ///
    /// * `pddb_dict` - the pddb dict holding all Dialogues for this Chat App
    /// * `pddb_key` - the pddb key holding a Dialogue
    pub fn read_only(pddb_dict: &str, pddb_key: Option<&str>) -> Self {
        let chat = Chat::new("_Chat Read_", "unused", None, None, None, None);
        chat.dialogue_set(pddb_dict, pddb_key).unwrap();
        chat
    }

    /// Set the flags on an Author of the current Dialogue, by name. The Author is added if
    /// absent, so an app may flag its local author before its first post; the change is
    /// display-only until the app sends ChatOp::DialogueSave.
    ///
    /// # Arguments
    ///
    /// * `author` - the (external) name of the Author
    /// * `flags` - the AuthorFlag set (e.g. AuthorFlag::Right to right-align own bubbles)
    pub fn author_flags_set(&self, author: &str, flags: EnumSet<AuthorFlag>) -> Result<(), Error> {
        let af = AuthorFlags { author: author.to_string(), flags: flags.as_u16() };
        match Buffer::into_buf(af) {
            Ok(buf) => buf.send(self.cid, ChatOp::AuthorFlagsSet as u32).map(|_| ()),
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Set the current Dialogue
    ///
    /// # Arguments
    ///
    /// * `pddb_dict` - the pddb dict holding all Dialogues for this Chat App
    /// * `pddb_key` - the pddb key holding a Dialogue
    pub fn dialogue_set(&self, pddb_dict: &str, pddb_key: Option<&str>) -> Result<(), Error> {
        let dialogue =
            api::Dialogue { dict: String::from(pddb_dict), key: pddb_key.map(|key| String::from(key)) };
        match Buffer::into_buf(dialogue) {
            Ok(buf) => buf.send(self.cid, ChatOp::DialogueSet as u32).map(|_| ()),
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Show some user help
    pub fn help(&self) {
        xous::send_message(self.cid, xous::Message::new_scalar(ChatOp::Help as usize, 0, 0, 0, 0))
            .map(|_| ())
            .expect("failed to get help");
    }

    /// Add a new MenuItem to the App menu
    ///
    /// # Arguments
    ///
    /// * `item` - an item action not handled by the Chat UI
    pub fn menu_add(&self, item: MenuItem) -> Result<(), Error> {
        match Buffer::into_buf(item) {
            Ok(buf) => buf.send(self.cid, ChatOp::MenuAdd as u32).map(|_| ()),
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Add a new Post to the current Dialogue
    ///
    /// note: posts are sorted by timestamp, so:
    /// - `post_add` at beginning or end is fast (middle triggers a binary partition)
    /// - if adding multiple posts then add oldest/newest last!
    ///
    /// # Arguments
    ///
    /// * `author` - the name of the Author of the Post
    /// * `timestamp` - the timestamp of the Post
    /// * `text` - the text content of the Post
    /// * `attach_url` - a url of an attachment (image for example)
    pub fn post_add(
        &self,
        author: &str,
        timestamp: u64,
        text: &str,
        attach_url: Option<&str>,
    ) -> Result<(), Error> {
        self.post_add_with_header(author, timestamp, text, None, attach_url)
    }

    /// Add a new Post with author chrome: a Bold line drawn above the
    /// bubble, outside its border (ISSUES r5 row 15). The header must
    /// come from the app's authenticated identity data, never from the
    /// message body; the Chat UI renders it in a separate TextView so
    /// no body text can imitate it. `header == None` behaves exactly
    /// like `post_add`.
    ///
    /// # Arguments
    ///
    /// * `author` - the name of the Author of the Post
    /// * `timestamp` - the timestamp of the Post
    /// * `text` - the text content of the Post
    /// * `header` - optional chrome line drawn Bold above the bubble
    /// * `attach_url` - a url of an attachment (image for example)
    pub fn post_add_with_header(
        &self,
        author: &str,
        timestamp: u64,
        text: &str,
        header: Option<&str>,
        attach_url: Option<&str>,
    ) -> Result<(), Error> {
        self.post_add_full(author, timestamp, text, header, None, false, attach_url)
    }

    /// Add a new Post with the full set of optional chrome: a Bold
    /// header line above the bubble, a Regular footer line below it
    /// (ISSUES r6 row 20), or `center == true` for a centered,
    /// borderless, box-free system row instead of a bubble at all
    /// (ISSUES r6 rows 21/22, a date separator or the message-request
    /// instruction bar). Header and footer must come from the app's
    /// authenticated identity/state data, never from the message body;
    /// the Chat UI renders each in a separate TextView so no body text
    /// can imitate them. `header == None && footer == None && center
    /// == false` behaves exactly like `post_add`.
    ///
    /// # Arguments
    ///
    /// * `author` - the name of the Author of the Post
    /// * `timestamp` - the timestamp of the Post
    /// * `text` - the text content of the Post
    /// * `header` - optional chrome line drawn Bold above the bubble
    /// * `footer` - optional chrome line drawn Regular below the bubble
    /// * `center` - true for a centered, borderless, box-free row
    /// * `attach_url` - a url of an attachment (image for example)
    pub fn post_add_full(
        &self,
        author: &str,
        timestamp: u64,
        text: &str,
        header: Option<&str>,
        footer: Option<&str>,
        center: bool,
        attach_url: Option<&str>,
    ) -> Result<(), Error> {
        let post = api::PostWithHeader {
            dialogue_id: String::new(),
            author: String::from(author),
            timestamp,
            text: String::from(text),
            attach_url: attach_url.map(String::from),
            header: header.map(String::from),
            footer: footer.map(String::from),
            center,
        };
        match Buffer::into_buf(post) {
            Ok(buf) => buf.send(self.cid, ChatOp::PostAddWithHeader as u32).map(|_| ()),
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Delete a Post from the current Dialogue
    ///
    /// # Arguments
    ///
    /// * `index` - the index of the Post to delete.
    pub fn post_del(&self, index: usize) -> Result<(), Error> {
        xous::send_message(self.cid, xous::Message::new_scalar(ChatOp::PostDel as usize, index, 0, 0, 0))
            .map(|_| ())
            .expect("failed to delete Pose {index}");
        Ok(())
    }

    /// Returns Some(index) of a matching Post by Author and Timestamp, or None
    ///
    /// # Arguments
    ///
    /// * `timestamp` - the Post timestamp criteria
    /// * `author` - the Post Author criteria
    ///
    /// Error if unable to send the msg to the Chat UI server
    pub fn post_find(&self, author: &str, timestamp: u64) -> Result<Option<usize>, Error> {
        let mut find = Find { author: String::new(), timestamp, key: None };
        find.author.push_str(author);
        match Buffer::into_buf(find) {
            Ok(mut buf) => match buf.lend_mut(self.cid, ChatOp::PostFind as u32) {
                Ok(..) => {
                    find = buf.to_original::<api::Find, _>().unwrap();
                    Ok(find.key)
                }
                Err(_) => Err(xous::Error::InternalError),
            },
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Set various status flags on a Post in the current Dialogue
    ///
    /// TODO: not implemented
    pub fn post_flag(&self, _key: &str) -> Result<(), Error> {
        log::warn!("not implemented");
        Err(xous::Error::InternalError)
    }

    /// Redraw our Chat UI.
    pub fn redraw(&self) {
        xous::send_message(self.cid, xous::Message::new_scalar(ChatOp::GamRedraw as usize, 0, 0, 0, 0))
            .map(|_| ())
            .expect("failed to Redraw Chat UI");
    }

    /// Set the four icontray slot labels, replacing the default "F1".."F4"
    pub fn set_icontray_labels(&self, labels: [&str; 4]) -> Result<(), Error> {
        let il = IcontrayLabels { labels: labels.map(String::from) };
        match Buffer::into_buf(il) {
            Ok(buf) => buf.send(self.cid, ChatOp::SetIcontrayLabels as u32).map(|_| ()),
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Opt this app's GAM context in or out of the IMEF's menu mode.
    ///
    /// # Arguments
    ///
    /// * `on` - true: F1-F4 and the arrows arrive as control strings on the
    ///   input line (dropped by the Chat UI; the app still sees F-keys via
    ///   the rawkeys fanout) and nothing is spliced into the compose line.
    ///   false (the default): F1-F4 insert the icontray slot label into the
    ///   compose line and arrows move the insertion point (mtxchat behavior).
    ///
    /// Opt-in: apps that never call this are unaffected.
    pub fn set_imef_menu_mode(&self, on: bool) -> Result<(), Error> {
        xous::send_message(
            self.cid,
            xous::Message::new_scalar(ChatOp::SetImefMenuMode as usize, on as usize, 0, 0, 0),
        )
        .map(|_| ())
    }

    /// Set the glyph style and framing of Dialogue posts.
    ///
    /// # Arguments
    ///
    /// * `style` - glyph style for post text
    /// * `bubbles` - true for the default look (bordered, rounded, 80%-width
    ///   bubbles); false for full-width borderless rows with a border drawn
    ///   only on the selected post
    ///
    /// Applies to the current and subsequent Dialogues.
    pub fn set_post_style(&self, style: GlyphStyle, bubbles: bool) -> Result<(), Error> {
        let ps = PostStyle { style: style as u32, bubbles };
        match Buffer::into_buf(ps) {
            Ok(buf) => buf.send(self.cid, ChatOp::SetPostStyle as u32).map(|_| ()),
            Err(_) => Err(xous::Error::InternalError),
        }
    }

    /// Set the status bar text.
    ///
    /// # Arguments
    ///
    /// `msg` - the text to show
    ///
    /// This method implements the latest recommendation of panicing on internal errors.
    pub fn set_status_text(&self, msg: &str) {
        let bm = BusyMessage { busy_msg: String::from(msg) };
        Buffer::into_buf(bm)
            .expect("internal error")
            .send(self.cid, ChatOp::SetStatusText as u32)
            .expect("internal error");
    }

    pub fn set_busy_state(&self, run: bool) {
        xous::send_message(
            self.cid,
            xous::Message::new_scalar(
                ChatOp::SetBusyAnimationState as usize,
                if run { 1 } else { 0 },
                0,
                0,
                0,
            ),
        )
        .map(|_| ())
        .expect("internal error");
    }

    /// Set status bar text when system is idle.
    /// This is a convenience so we don't have to track the ins/outs of busy/idle state
    /// and update the text, especially when we have multiple potential servers vying
    /// to set a busy state.
    pub fn set_status_idle_text(&self, msg: &str) {
        let bm = BusyMessage { busy_msg: String::from(msg) };
        Buffer::into_buf(bm)
            .expect("internal error")
            .send(self.cid, ChatOp::SetStatusIdleText as u32)
            .expect("internal error");
    }
}

/// Helper server that pumps the busy animation state until instructed to stop.
///
/// # Arguments
///
/// * `busy_bumper` - the server ID to use for the helper server
/// * `busy_bumper_cid` - the corresponding connection ID
/// * `chat_cid` - the CID to the main chat loop, used to initiate redraw events as necessary
/// * `run_busy_animation` - a shared `AtomicBool` which, when `true`, causes the loop to reschedule itself to
///   run.
pub fn busy_animator(
    busy_bumper: SID,
    busy_bumper_cid: CID,
    chat_cid: CID,
    run_busy_animation: Arc<AtomicBool>,
) {
    let tt = ticktimer_server::Ticktimer::new().unwrap();
    loop {
        let msg = xous::receive_message(busy_bumper).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(BusyAnimOp::Start) => {
                tt.sleep_ms(crate::BUSY_ANIMATION_RATE_MS).unwrap();
                xous::try_send_message(
                    busy_bumper_cid,
                    xous::Message::new_scalar(BusyAnimOp::Pump as usize, 0, 0, 0, 0),
                )
                .ok();
            }
            Some(BusyAnimOp::Pump) => {
                if run_busy_animation.load(Ordering::SeqCst) {
                    xous::try_send_message(
                        chat_cid,
                        xous::Message::new_scalar(ChatOp::UpdateBusy as usize, 0, 0, 0, 0),
                    )
                    .ok();
                    tt.sleep_ms(crate::BUSY_ANIMATION_RATE_MS).unwrap();
                    xous::try_send_message(
                        busy_bumper_cid,
                        xous::Message::new_scalar(BusyAnimOp::Pump as usize, 0, 0, 0, 0),
                    )
                    .ok();
                }
            }
            _ => {
                log::warn!("Unexpected message: {:?}", msg);
            }
        }
    }
}

/// The Chat UI server a manages a Chat UI to read a display and navigate a
/// series of Posts in a Dialogue stored in the pddb - and to Author a new
/// Post.
///
/// # Arguments
///
/// * `app_name` - registered with GAM
/// * `app_menu` - with menu items handled by the Chat App rather than the Chat UI
/// * `app_cid` - to accept messages from the Chat UI (see below)
/// * `post_opcode` - to handle a `MemoryMessage` containing a new outbound user Post
/// * `event_opcode` - to handle `ScalarMessage` representing a UI Event, such as F1 click, Left click, Top
///   Post, etc.
/// * `rawkeys_opcode` - to handle a raw-keystroke.
pub fn server(
    sid: SID,
    app_name: &'static str,
    app_menu: &'static str,
    app_cid: Option<CID>,
    opcode_post: Option<usize>,
    opcode_event: Option<usize>,
    opcode_rawkeys: Option<usize>,
    run_busy_animation: Arc<AtomicBool>,
    busy_bumper_cid: CID,
) -> ! {
    //log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("my PID is {}", xous::process::id());

    let mut ui = ui::Ui::new(sid, app_name, app_menu, app_cid, opcode_event);
    let mut dialogue_key = None;
    let mut allow_redraw = false;
    loop {
        let msg = xous::receive_message(sid).unwrap();
        log::debug!("got message {:?}", msg);
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(ChatOp::UpdateBusy) => {
                ui.redraw_busy().expect("CHAT couldn't redraw");
            }
            Some(ChatOp::UpdateBusyForced) => {
                ui.redraw_status_forced().expect("CHAT couldn't redraw");
            }
            Some(ChatOp::SetStatusText) => {
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                let s = buffer.to_original::<BusyMessage, _>().unwrap();
                ui.set_status_text(s.busy_msg.as_str());
            }
            Some(ChatOp::SetIcontrayLabels) => {
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                match buffer.to_original::<IcontrayLabels, _>() {
                    Ok(labels) => ui.set_icontray_labels(labels),
                    Err(e) => log::warn!("failed to deserialize IcontrayLabels: {:?}", e),
                }
            }
            Some(ChatOp::SetImefMenuMode) => msg_scalar_unpack!(msg, on, _, _, _, {
                ui.set_imef_menu_mode(on != 0);
            }),
            Some(ChatOp::SetPostStyle) => {
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                match buffer.to_original::<PostStyle, _>() {
                    Ok(ps) => ui.set_post_style(ps),
                    Err(e) => log::warn!("failed to deserialize PostStyle: {:?}", e),
                }
            }
            Some(ChatOp::SetStatusIdleText) => {
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                let s = buffer.to_original::<BusyMessage, _>().unwrap();
                ui.set_status_idle_text(s.busy_msg.as_str());
            }
            Some(ChatOp::SetBusyAnimationState) => msg_scalar_unpack!(msg, state, _, _, _, {
                if state != 0 {
                    ui.set_busy_state(true);
                    if !run_busy_animation.swap(true, Ordering::SeqCst) {
                        // only send off the Pump request on the transition from false->true; this causes the
                        // machine to run
                        xous::try_send_message(
                            busy_bumper_cid,
                            xous::Message::new_scalar(BusyAnimOp::Pump as usize, 0, 0, 0, 0),
                        )
                        .ok();
                    }
                } else {
                    run_busy_animation.store(false, Ordering::SeqCst);
                    ui.set_busy_state(false);
                }
            }),
            Some(ChatOp::DialogueSave) => {
                log::info!("ChatOp::DialogueSave");
                ui.dialogue_save().expect("failed to save Dialogue");
                ui.dialogue_read().expect("failed to read Dialogue");
                if allow_redraw {
                    ui.redraw().expect("CHAT couldn't redraw");
                }
            }
            Some(ChatOp::DialogueSet) => {
                log::info!("ChatOp::DialogueSet");
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                let dialogue = buffer.to_original::<Dialogue, _>().unwrap();
                dialogue_key = match dialogue.key {
                    Some(key) => Some(key.to_string()),
                    None => None,
                };
                ui.dialogue_set(dialogue.dict.as_str(), dialogue_key.as_deref());
            }
            Some(ChatOp::GamChangeFocus) => {
                log::info!("ChatOp::GamChangeFocus");
                xous::msg_scalar_unpack!(msg, new_state_code, _, _, _, {
                    let new_state = gam::FocusState::convert_focus_change(new_state_code);
                    match new_state {
                        gam::FocusState::Background => {
                            allow_redraw = false;
                        }
                        gam::FocusState::Foreground => {
                            allow_redraw = true;
                            ui.event(Event::Focus);
                        }
                    }
                })
            }
            Some(ChatOp::GamLine) => {
                log::info!("got ChatOp::GamLine");
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                let s = buffer.as_flat::<String, _>().unwrap();
                match s.as_str() {
                    "\u{0011}" => {}
                    "\u{0012}" => {}
                    "\u{0013}" => {}
                    "\u{0014}" => {}
                    "↑" => {}
                    "↓" => {}
                    "←" => {}
                    "→" => {}
                    _ => {
                        drop(buffer);
                        if let Some(cid) = app_cid {
                            if let Some(opcode) = opcode_post {
                                log::info!("Forwarding msg to Chat App: {:?}", msg);
                                msg.forward(cid, opcode).expect("failed to fwd msg");
                            }
                        }
                    }
                }
            }
            Some(ChatOp::GamRawkeys) => {
                log::info!("got ChatOp::GamRawkeys");
                xous::msg_scalar_unpack!(msg, k1, k2, k3, k4, {
                    log::info!("got Chat UI RawKey :{}:{}:{}:{}:", k1, k2, k3, k4);
                    match core::char::from_u32(k1 as u32).unwrap_or('\u{0000}') {
                        F1 => {
                            log::info!("click F1 : pull request welcome!");
                            ui.event(Event::F1);
                        }
                        F2 => {
                            log::info!("click F2 : pull request welcome!");
                            ui.event(Event::F2);
                        }
                        F3 => {
                            log::info!("click F3 : pull request welcome!");
                            ui.event(Event::F3);
                        }
                        F4 => {
                            log::info!("click F4 : pull request welcome!");
                            ui.event(Event::F4);
                        }
                        '↑' => {
                            log::info!("click ↑ : previous post");
                            ui.set_menu_mode(true); // ← & → activate menus
                            ui.post_select(POST_SELECTED_PREV);
                            ui.redraw().expect("failed to redraw chat");
                            ui.event(Event::Up);
                        }
                        '↓' => {
                            log::info!("click ↓ : next post");
                            ui.post_select(POST_SELECTED_NEXT);
                            ui.redraw().expect("failed to redraw chat");
                            ui.event(Event::Down);
                        }
                        '←' => {
                            log::info!("click ← : raise app menu");
                            if ui.get_menu_mode() {
                                ui.raise_app_menu();
                            }
                            ui.event(Event::Left);
                        }
                        '→' => {
                            log::info!("click → : raise msg menu : pull request welcome!");
                            if ui.get_menu_mode() {
                                ui.raise_msg_menu();
                            }
                            ui.event(Event::Right);
                        }
                        _ => {
                            ui.set_menu_mode(false); // ← & → move input cursor
                        }
                    }
                });
                if let Some(cid) = app_cid {
                    if let Some(opcode) = opcode_rawkeys {
                        log::info!("Forwarding msg to Chat App: {:?}", msg);
                        msg.forward(cid, opcode).expect("failed to fwd rawkey");
                    }
                }
            }
            Some(ChatOp::Help) => {
                log::info!("ChatOp::Help");
                ui.help();
            }
            Some(ChatOp::GamRedraw) => {
                log::info!("ChatOp::GamRedraw");
                if allow_redraw {
                    ui.redraw().expect("CHAT couldn't redraw");
                }
            }
            Some(ChatOp::PostAdd) => {
                log::info!("ChatOp::PostAdd");
                match dialogue_key {
                    Some(ref dialogue_id) => {
                        let buffer =
                            unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                        match buffer.to_original::<api::Post, _>() {
                            Ok(post) => ui
                                .post_add(
                                    &dialogue_id,
                                    post.author.as_str(),
                                    post.timestamp,
                                    post.text.as_str(),
                                    None,
                                    None,
                                    false,
                                    None, // TODO implement
                                )
                                .unwrap(),
                            Err(e) => log::warn!("failed to deserialize Post: {:?}", e),
                        }
                    }
                    None => log::warn!("failed to PostAdd with Dialogue == None"),
                }
            }
            Some(ChatOp::PostAddWithHeader) => {
                log::info!("ChatOp::PostAddWithHeader");
                match dialogue_key {
                    Some(ref dialogue_id) => {
                        let buffer =
                            unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                        match buffer.to_original::<api::PostWithHeader, _>() {
                            Ok(post) => ui
                                .post_add(
                                    &dialogue_id,
                                    post.author.as_str(),
                                    post.timestamp,
                                    post.text.as_str(),
                                    post.header.as_deref(),
                                    post.footer.as_deref(),
                                    post.center,
                                    None, // TODO implement attachments
                                )
                                .unwrap(),
                            Err(e) => log::warn!("failed to deserialize PostWithHeader: {:?}", e),
                        }
                    }
                    None => log::warn!("failed to PostAddWithHeader with Dialogue == None"),
                }
            }
            Some(ChatOp::PostDel) => {
                xous::msg_scalar_unpack!(msg, index, _, _, _, {
                    log::info!("ChatOp::PostDel {index}");
                    ui.post_del(index).expect("failed to delete post {index}");
                });
            }
            Some(ChatOp::PostFind) => {
                log::info!("ChatOp::PostAdd");
                let mut buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                if let Ok(mut find) = buffer.to_original::<Find, _>() {
                    find.key = ui.post_find(find.author.as_str(), find.timestamp);
                    buffer.replace(find).expect("couldn't serialize return");
                } else {
                    log::warn!("failed to serialize Find");
                }
            }
            Some(ChatOp::PostFlag) => {
                log::warn!("ChatOp::PostFlag not implemented");
            }
            Some(ChatOp::MenuAdd) => {
                log::warn!("ChatOp::MenuAdd not implemented");
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                if let Ok(menu_item) = buffer.to_original::<MenuItem, _>() {
                    ui.menu_add(menu_item);
                } else {
                    log::warn!("failed to deserialize MenuItem");
                }
            }
            Some(ChatOp::AuthorFlagsSet) => {
                let buffer = unsafe { Buffer::from_memory_message(msg.body.memory_message().unwrap()) };
                match buffer.to_original::<AuthorFlags, _>() {
                    Ok(af) => ui.author_flags_set(&af.author, EnumSet::from_u16(af.flags)),
                    Err(e) => log::warn!("failed to deserialize AuthorFlags: {:?}", e),
                }
            }
            Some(ChatOp::Quit) => {
                log::error!("got Quit");
                break;
            }
            _ => log::warn!("got unknown message"),
        }
        log::trace!("reached bottom of main loop");
    }
    // clean up our program
    log::error!("main loop exit, destroying servers");
    xous::destroy_server(sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}

pub fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().try_into().unwrap() }

/// "context-free" (cf) communication with the chat object.
/// This is accomplished by making a copy of the connection to the chat server.
pub fn cf_set_status_text(chat_cid: xous::CID, msg: &str) {
    let bm = BusyMessage { busy_msg: String::from(msg) };
    Buffer::into_buf(bm)
        .expect("internal error")
        .send(chat_cid, ChatOp::SetStatusText as u32)
        .expect("internal error");
}

pub fn cf_set_busy_state(chat_cid: xous::CID, run: bool) {
    xous::send_message(
        chat_cid,
        xous::Message::new_scalar(ChatOp::SetBusyAnimationState as usize, if run { 1 } else { 0 }, 0, 0, 0),
    )
    .map(|_| ())
    .expect("internal error");
}
