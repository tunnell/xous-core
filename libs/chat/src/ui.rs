use std::cmp::min;
use std::fmt::Write as TextWrite;
use std::io::{Error, ErrorKind, Read, Write};

use blitstr2::GlyphStyle;
use dialogue::{Dialogue, post::Post};
use gam::{MenuMatic, UxRegistration, menu_matic};
use locales::t;
use modals::Modals;
use ticktimer_server::Ticktimer;
use ux_api::minigfx::*;
use ux_api::service::api::*;
use xous::{CID, MessageEnvelope};
use xous_names::XousNames;

use super::*;
//use crate::{ChatOp, Dialogue, Event, Post, CHAT_SERVER_NAME};
use crate::icontray::Icontray;

pub const BUSY_ANIMATION_RATE_MS: usize = 200;

/// Variables that define the visual properties of the layout
pub struct VisualProperties {
    pub canvas: Gid,
    pub total_screensize: Point,
    pub layout_screensize: Point,
    /// height of the status bar. This is subtracted from screensize.
    pub status_height: u16,
    pub bubble_width: u16,
    pub margin: Point,        // margin to edge of canvas
    pub bubble_margin: Point, // margin of text in bubbles
    pub bubble_radius: u16,
    pub bubble_space: isize, // spacing between text bubbles
    /// glyph style for post text
    pub style: GlyphStyle,
    /// true: bordered rounded bubbles at bubble_width; false: full-width
    /// borderless rows with a border only on the selected post
    pub bubbles: bool,
}
#[allow(dead_code)]
pub(crate) struct Ui {
    // optional structures that indicate new input to the Chat loop per iteration
    // an input string
    pub input: Option<String>,
    // messages from other servers
    msg: Option<MessageEnvelope>,

    // Pddb connection
    pddb: pddb::Pddb,
    pddb_dict: Option<String>,
    pddb_key: Option<String>,
    dialogue: Option<Dialogue>,

    // Callbacks:
    // callback to our own server
    self_cid: CID,
    // optional SID of the "Owner" Chat App to receive UI-events
    app_cid: Option<CID>,
    // optional opcode ID to process UI-event msgs
    opcode_event: Option<usize>,

    gam: gam::Gam,
    modals: Modals,
    tt: Ticktimer,

    /// These variables are managed exclusively by the layout routine.
    /// the selected post is highlighted onscreen and the focus of the msg menu
    layout_selected: Option<usize>,
    /// the range of posts that are currently drawable. This was originally implemented as
    /// an Option<Range>, but we need to be able to do RangeInclusive and Reversed ranges.
    /// The RangeBounds trait isn't object-safe, so we can't Box/dyn it either...
    /// So, instead, we turn the ranges into a Vec and operate from there...
    layout_range: Vec<usize>,
    /// layout post bubbles on the screen from top-down or bottom-up
    layout_topdown: bool,

    /// TextView for the status bar. This encapsulates the state of the busy animation, and the text within.
    status_tv: TextView,
    /// Track the last time we update the status bar; use this avoid double-updating busy animations
    status_last_update_ms: u64,
    /// The default message to show when we exit a busy state
    status_idle_text: String,

    vp: VisualProperties,

    // variables that define a menu
    menu_mode: bool,
    app_menu: String,
    menu_mgr: MenuMatic,

    // tracks the IMEF menu-mode state of our GAM context. The GAM only
    // exposes a toggle (Opcode::ToggleMenuMode), not a setter, so this
    // shadow of the GAM-side flag (default false, contexts.rs) is what
    // makes set_imef_menu_mode idempotent: toggling twice would re-enable
    // prediction splicing into the compose line.
    imef_menu_mode: bool,

    // connection to this app's icontray server, for label updates
    icontray_cid: CID,

    // our security token for making changes to our record on the GAM
    token: [u32; 4],
}

#[allow(dead_code)]
impl Ui {
    pub(crate) fn new(
        sid: xous::SID,
        app_name: &'static str,
        app_menu: &'static str,
        app_cid: Option<xous::CID>,
        opcode_event: Option<usize>,
    ) -> Self {
        let xns = XousNames::new().unwrap();
        let gam = gam::Gam::new(&xns).expect("can't connect to GAM");

        // the icontray server name is per-app: two chat apps in one image each spawn one, and a
        // shared name would fail the second registration
        let icontray_name = format!("_{} icon tray plugin_", app_name);
        let token = gam
            .register_ux(UxRegistration {
                app_name: String::from(app_name),
                ux_type: gam::UxType::Chat,
                predictor: Some(icontray_name.clone()),
                listener: sid.to_array(), /* note disclosure of our SID to the GAM -- the secret is now
                                           * shared with the GAM! */
                redraw_id: ChatOp::GamRedraw as u32,
                gotinput_id: Some(ChatOp::GamLine as u32),
                audioframe_id: None,
                rawkeys_id: Some(ChatOp::GamRawkeys as u32),
                focuschange_id: Some(ChatOp::GamChangeFocus as u32),
            })
            .expect("couldn't register Ux context for chat")
            .unwrap();
        let xns = XousNames::new().unwrap();
        let modals = Modals::new(&xns).unwrap();
        let canvas = gam.request_content_canvas(token).expect("couldn't get content canvas");
        let screensize = gam.get_canvas_bounds(canvas).expect("couldn't get dimensions of content canvas");
        // TODO this is a stub - implement F1-4 actions and autocompletes
        let _icontray =
            Icontray::new(icontray_name.clone(), Some(xous::connect(sid).unwrap()), ["F1", "F2", "F3", "F4"]);
        let icontray_cid = xns.request_connection_blocking(&icontray_name).expect("can't connect to icontray");
        let menu_mgr = menu_matic(Vec::<MenuItem>::new(), app_menu, Some(xous::create_server().unwrap()))
            .expect("couldn't create MenuMatic manager");
        let pddb = pddb::Pddb::new();
        pddb.try_mount();

        // setup the initial status bar contents
        let margin = Point::new(4, 4);
        let status_height = gam.glyph_height_hint(GlyphStyle::Regular).unwrap() as u16;
        let mut status_tv = TextView::new(
            canvas,
            TextBounds::BoundingBox(Rectangle::new(
                Point::new(0, 0),
                Point::new(screensize.x, status_height as _),
            )),
        );
        status_tv.style = GlyphStyle::Regular;
        status_tv.margin = margin;
        status_tv.draw_border = false;
        status_tv.clear_area = true;
        status_tv.margin = Point::new(0, 0);
        write!(status_tv, "{}", t!("chat.status.initial", locales::LANG).to_string()).ok();
        let tt = ticktimer_server::Ticktimer::new().unwrap();
        let status_last_update_ms = tt.elapsed_ms();
        let bubble_properties = VisualProperties {
            canvas,
            total_screensize: screensize,
            layout_screensize: Point::new(screensize.x, screensize.y - status_height as isize),
            status_height,
            bubble_width: ((screensize.x / 5) * 4) as u16, // 80% width for the text bubbles
            margin: Point::new(4, 4),
            bubble_margin: Point::new(4, 4),
            bubble_radius: 4,
            bubble_space: 4,
            style: GlyphStyle::Regular,
            bubbles: true,
        };
        Ui {
            input: None,
            msg: None,
            pddb,
            pddb_dict: None,
            pddb_key: None,
            dialogue: None,
            self_cid: xous::connect(sid).unwrap(),
            app_cid,
            opcode_event,
            gam,
            modals,
            tt,
            status_tv,
            status_last_update_ms,
            layout_selected: None,
            layout_range: Vec::new(),
            layout_topdown: false,
            vp: bubble_properties,
            menu_mode: true,
            app_menu: app_menu.to_owned(),
            menu_mgr,
            imef_menu_mode: false,
            icontray_cid,
            token,
            status_idle_text: t!("chat.status.initial", locales::LANG).to_string(),
        }
    }

    /// Read the current Dialogue from pddb
    pub fn dialogue_read(&mut self) -> Result<(), Error> {
        match (&self.pddb_dict, &self.pddb_key) {
            (Some(dict), Some(key)) => {
                match self.pddb.get(&dict, &key, None, true, false, None, None::<fn()>) {
                    Ok(mut pddb_key) => {
                        let mut bytes = [0u8; dialogue::MAX_BYTES + 2];
                        match pddb_key.read(&mut bytes) {
                            Ok(pos) => {
                                let archive = unsafe {
                                    rkyv::access_unchecked::<dialogue::ArchivedDialogue>(&bytes[..pos])
                                };
                                self.dialogue =
                                    match rkyv::deserialize::<Dialogue, rkyv::rancor::Error>(archive) {
                                        Ok(dialogue) => {
                                            // show most recent posts onscreen
                                            self.layout_selected = dialogue.post_last();
                                            self.layout_range.clear();
                                            self.layout_topdown = false;
                                            Some(dialogue)
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "failed to deserialize Dialogue {}:{} {}",
                                                dict,
                                                key,
                                                e
                                            );
                                            None
                                        }
                                    };
                                log::debug!("get '{}' = '{:?}'", key, self.dialogue);
                            }
                            Err(e) => log::warn!("failed to read {}: {e}", key),
                        }
                    }
                    Err(e) => {
                        log::warn!("failed to get {}: {e}", key);
                        return Err(Error::new(ErrorKind::InvalidData, "missing"));
                    }
                }
                Ok(())
            }
            _ => {
                log::warn!("missing pddb dict or key");
                Err(Error::new(ErrorKind::InvalidData, "missing"))
            }
        }
    }

    /// Save the current Dialogue to pddb
    pub fn dialogue_save(&self) -> Result<(), Error> {
        match (&self.dialogue, &self.pddb_dict, &self.pddb_key) {
            (Some(dialogue), Some(dict), Some(key)) => {
                let hint = Some(dialogue::MAX_BYTES + 2);
                match self.pddb.get(&dict, &key, None, true, true, hint, None::<fn()>) {
                    Ok(mut pddb_key) => {
                        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(dialogue).unwrap();
                        match pddb_key.write(&bytes) {
                            Ok(len) => {
                                self.pddb.sync().ok();
                                log::info!("Wrote {} bytes to {}:{}", len, dict, key);
                            }
                            Err(e) => {
                                log::warn!("Error writing {}:{}: {:?}", dict, key, e);
                            }
                        }
                    }
                    Err(e) => log::warn!("failed to create {}:{}\n{}", dict, key, e),
                }
                Ok(())
            }
            _ => {
                log::warn!("missing dict, key or dialogue");
                Ok(())
            }
        }
    }

    /// Set the current Dialogue
    ///
    /// # Arguments
    ///
    /// * `pddb_dict` - the pddb dict holding all Dialogues for this Chat App
    /// * `pddb_key` - the pddb key holding a Dialogue
    pub fn dialogue_set(&mut self, pddb_dict: &str, pddb_key: Option<&str>) {
        self.pddb_dict = Some(pddb_dict.to_string());
        self.pddb_key = pddb_key.map(|key| key.to_string());
        // drop layout state from the previous dialogue: stale indices would pass the consistency
        // check in layout() and suppress the range recompute against the new post list
        self.layout_selected = None;
        self.layout_range.clear();
        if self.pddb_key.is_none() {
            self.dialogue_modal();
        }
        log::info!("Dialogue set to {:?}:{:?}", self.pddb_dict, self.pddb_key);
        match self.dialogue_read() {
            Ok(_) => {
                log::info!("read dialogue {:?}:{:?}", self.pddb_dict, self.pddb_key);
                self.redraw().expect("couldn't redraw screen");
            }
            Err(_) => {
                if let Some(key) = &self.pddb_key {
                    self.dialogue = Some(Dialogue::new(&key));
                    match self.dialogue_save() {
                        Ok(_) => log::info!("Dialogue created {}:{}", pddb_dict, key),
                        Err(e) => {
                            log::warn!("Failed to create Dialogue {}:{} : {e}", pddb_dict, key)
                        }
                    }
                }
            }
        }
    }

    /// Present a Modal to select Dialogue from pddb
    ///
    /// typically called in offline mode
    ///
    /// TODO move non-dialogue keys elsewhere
    pub fn dialogue_modal(&mut self) {
        if let Some(dict) = &self.pddb_dict {
            match self.pddb.list_keys(&dict, None) {
                Ok(keys) => {
                    if keys.len() > 0 {
                        self.modals
                            .add_list(keys.iter().map(|s| s.as_str()).collect())
                            .expect("failed modal add_list");
                        self.pddb_key =
                            self.modals.get_radiobutton(t!("chat.dialogue_title", locales::LANG)).ok();
                        log::info!("selected dialogue {}:{:?}", dict, self.pddb_key);
                    } else {
                        self.modals
                            .show_notification(t!("chat.dict_empty", locales::LANG), None)
                            .expect("notification failed");
                    }
                }
                Err(e) => log::warn!("failed to list pddb keys: {e}"),
            }
        }
    }

    /// Show some user help
    pub fn help(&self) {
        self.modals
            .show_notification(t!("chat.help.navigation", locales::LANG), None)
            .expect("notification failed");
    }

    /// Add a new MenuItem to the App menu
    ///
    /// # Arguments
    ///
    /// * `item` - an item action not handled by the Chat UI
    pub fn menu_add(&self, item: MenuItem) { self.menu_mgr.add_item(item); }

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
    /// * `header` - optional author chrome drawn Bold above the bubble
    /// * `footer` - optional chrome drawn Regular below the bubble (ISSUES r6 row 20)
    /// * `center` - true for a centered, borderless, box-free system row (ISSUES r6 rows 21/22)
    /// * `attach_url` - a url of an attachment (image for example)
    pub fn post_add(
        &mut self,
        dialogue_id: &str,
        author: &str,
        timestamp: u64,
        text: &str,
        header: Option<&str>,
        footer: Option<&str>,
        center: bool,
        attach_url: Option<&str>,
    ) -> Result<(), Error> {
        match (&self.pddb_key, &mut self.dialogue) {
            (Some(pddb_key), Some(ref mut dialogue)) => {
                if dialogue_id.len() == 0 || pddb_key.eq(&dialogue_id) {
                    dialogue
                        .post_add(
                            author,
                            timestamp,
                            text,
                            header,
                            footer,
                            center,
                            attach_url,
                            Some((&self.vp, &self.gam)),
                        )
                        .unwrap();
                } else {
                    log::warn!(
                        "dropping Post as dialogue_id does not match pddb_key: '{}' vs '{}'",
                        pddb_key,
                        dialogue_id
                    );
                }
            }
            (None, _) => log::warn!("no pddb_key set to match dialogue_id"),
            (_, None) => log::warn!("no Dialogue available to add Post"),
        }
        Ok(())
    }

    /// Delete a Post from the current Dialogue
    ///
    /// TODO: implement post_delete()

    pub fn post_del(&self, _index: usize) -> Result<(), Error> {
        log::warn!("not implemented");
        Err(Error::new(ErrorKind::Other, "not implemented"))
    }

    /// Returns Some(index) of a matching Post by Author and Timestamp, or None
    ///
    /// # Arguments
    ///
    /// * `timestamp` - the Post timestamp criteria
    /// * `author` - the Post Author criteria
    pub fn post_find(&self, author: &str, timestamp: u64) -> Option<usize> {
        match &self.dialogue {
            Some(dialogue) => dialogue.post_find(author, timestamp),
            None => None,
        }
    }

    /// Return Some<Post> from the current Dialogue, or None
    ///
    /// # Arguments
    ///
    /// * `index` - index of the Post to retrieve

    /// Set the flags on an Author of the current Dialogue; a warning no-op without one.
    pub fn author_flags_set(&mut self, author: &str, flags: EnumSet<AuthorFlag>) {
        match self.dialogue.as_mut() {
            Some(dialogue) => dialogue.author_flags_set(author, flags),
            None => log::warn!("no Dialogue available to set Author flags"),
        }
    }
    pub fn post_get(&self, index: usize) -> Option<&Post> {
        match &self.dialogue {
            Some(dialogue) => dialogue.post_get(index),
            None => None,
        }
    }

    /// Set various status flags on a Post in the current Dialogue
    ///
    /// TODO: not implemented
    pub fn post_flag(&self, _key: u32) -> Result<(), Error> {
        log::warn!("not implemented");
        Err(Error::new(ErrorKind::Other, "not implemented"))
    }

    /// Set the Selected Post to an arbitrary index
    ///
    /// # Arguments
    ///
    /// * `index` - POST_SELECT_NEXT or POST_SELECT_PREV or an arbitraty index
    pub fn post_select(&mut self, index: usize) {
        self.layout_selected = match &self.dialogue {
            Some(dialogue) => {
                match dialogue.post_last() {
                    Some(last_post) => {
                        match (index, self.layout_selected) {
                            (POST_SELECTED_NEXT, Some(selected)) => {
                                if selected >= last_post {
                                    self.event(Event::Bottom);
                                    Some(last_post)
                                } else {
                                    Some(selected + 1)
                                }
                            }
                            (POST_SELECTED_PREV, Some(selected)) => {
                                if selected == 0 {
                                    self.event(Event::Top);
                                    Some(selected)
                                } else {
                                    Some(selected - 1)
                                }
                            }
                            (index, _) => Some(min(index, last_post)), // arbitrary post
                        }
                    }
                    None => None,
                }
            }
            None => None,
        }
    }

    pub fn get_menu_mode(&self) -> bool { self.menu_mode }

    pub fn set_menu_mode(&mut self, menu_mode: bool) { self.menu_mode = menu_mode; }

    /// Set the IMEF menu-mode of this app's GAM context (see
    /// ChatOp::SetImefMenuMode). The GAM only offers a toggle, so the
    /// tracked state gates the call: repeated requests for the same
    /// state are no-ops (see `imef_toggle_needed`).
    pub fn set_imef_menu_mode(&mut self, on: bool) {
        if imef_toggle_needed(self.imef_menu_mode, on) {
            match self.gam.toggle_menu_mode(self.token) {
                Ok(()) => self.imef_menu_mode = on,
                Err(e) => log::warn!("failed to toggle IMEF menu mode: {:?}", e),
            }
        }
    }

    /// Send a xous scalar message with an Event to the Chat App cid/opcode
    ///
    /// # Arguments
    ///
    /// * `event` - the type of event to send
    ///
    /// Error when `app_cid` == None or `opcode_event` == None
    pub fn event(&self, event: Event) {
        log::info!("Event {:?}", event);
        match (self.app_cid, self.opcode_event) {
            (Some(cid), Some(opcode)) => match xous::send_message(
                cid,
                xous::Message::new_scalar(opcode as usize, event as usize, 0, 0, 0),
            ) {
                Ok(_) => log::info!("sent event msg"),
                Err(e) => log::warn!("failed to send event msg: {:?}", e),
            },
            _ => log::warn!("missing cid or event opcode"),
        }
    }

    /// Clear the screen area, not including the status bar
    fn clear_area(&self) {
        self.gam
            .draw_rectangle(
                self.vp.canvas,
                Rectangle::new_with_style(
                    Point::new(0, self.vp.status_height as isize),
                    self.vp.total_screensize,
                    DrawStyle { fill_color: Some(PixelColor::Light), stroke_color: None, stroke_width: 0 },
                ),
            )
            .expect("can't clear canvas area");
    }

    /// Show the App Menu (← key)
    pub(crate) fn raise_app_menu(&mut self) {
        self.gam.raise_menu(&self.app_menu).expect("couldn't raise our submenu");
        log::info!("raised app menu");
    }

    /// Show the Msg Menu (→ key)
    pub(crate) fn raise_msg_menu(&mut self) {
        log::warn!("msg menu not implemented - pull-requests welcome");
    }

    /// Redraw posts on the screen.
    ///
    /// Up to three attempts are made to layout the Posts:
    /// * ensuring the selected post is fully visible, and
    /// * best use of the screen is achieved
    pub(crate) fn redraw(&mut self) -> Result<(), xous::Error> {
        if self.dialogue.is_some() {
            self.layout().expect("layout failed to execute");
            self.status_last_update_ms = self.tt.elapsed_ms();
        } else {
            self.clear_area(); // no dialogue so clear screen
        }
        log::trace!("chat app redraw##");
        self.gam.redraw().expect("couldn't redraw screen");
        Ok(())
    }

    /// Update the busy state. Does not touch any other aspect of the screen layout.
    pub(crate) fn redraw_busy(&mut self) -> Result<(), xous::Error> {
        let curtime = self.tt.elapsed_ms();
        if curtime - self.status_last_update_ms > BUSY_ANIMATION_RATE_MS as u64 {
            self.gam.post_textview(&mut self.status_tv)?;
            self.gam.redraw().expect("couldn't redraw screen");
            self.status_last_update_ms = curtime;
        }
        Ok(())
    }

    /// Update the status bar, without any throttling
    pub(crate) fn redraw_status_forced(&mut self) -> Result<(), xous::Error> {
        self.gam.post_textview(&mut self.status_tv)?;
        self.gam.redraw().expect("couldn't redraw screen");
        let curtime = self.tt.elapsed_ms();
        self.status_last_update_ms = curtime;
        Ok(())
    }

    /// Returns `true` if the status bar is currently set for the busy animation
    pub(crate) fn is_busy(&self) -> bool { self.status_tv.busy_animation_state.is_some() }

    /// Set the glyph style and framing of Dialogue posts, dropping the
    /// cached post heights so the next layout measures with the new style.
    pub(crate) fn set_post_style(&mut self, ps: PostStyle) {
        self.vp.style = GlyphStyle::from(ps.style as usize);
        self.vp.bubbles = ps.bubbles;
        self.vp.bubble_width = if ps.bubbles {
            ((self.vp.total_screensize.x / 5) * 4) as u16 // 80% width for the text bubbles
        } else {
            (self.vp.total_screensize.x - 2 * self.vp.margin.x) as u16
        };
        if let Some(dialogue) = self.dialogue.as_mut() {
            for post in dialogue.posts_mut() {
                post.bounding_box = None;
                post.header_box = None;
                post.footer_box = None;
            }
        }
    }

    /// Forward the four icontray slot labels to this app's icontray server
    pub fn set_icontray_labels(&self, labels: IcontrayLabels) {
        match Buffer::into_buf(labels) {
            Ok(buf) => buf
                .send(self.icontray_cid, crate::icontray::OP_SET_LABELS as u32)
                .map(|_| ())
                .unwrap_or_else(|e| log::warn!("couldn't send icontray labels: {:?}", e)),
            Err(e) => log::warn!("couldn't serialize icontray labels: {:?}", e),
        }
    }

    /// Set the status bar text
    pub(crate) fn set_status_text(&mut self, msg: &str) {
        self.status_tv.clear_str();
        write!(self.status_tv, "{}", msg).ok();
        xous::send_message(self.self_cid, xous::Message::new_scalar(ChatOp::UpdateBusy as usize, 0, 0, 0, 0))
            .ok();
    }

    /// Sets the status bar to animate the busy animation
    pub(crate) fn set_busy_state(&mut self, run: bool) {
        if run {
            self.status_tv.busy_animation_state = Some(0); // the "glitch" to 0 is intentional, gives an indicator that a new op has started
        } else {
            if self.status_tv.busy_animation_state.take().is_some() {
                self.status_tv.clear_str();
                write!(self.status_tv, "{}", self.status_idle_text).ok();
                // force the update, to ensure the idle state text is actually rendered
                xous::send_message(
                    self.self_cid,
                    xous::Message::new_scalar(ChatOp::UpdateBusyForced as usize, 0, 0, 0, 0),
                )
                .ok();
            }
        }
    }

    /// Set the default idle text. Does *not* cause a redraw. If you need
    /// an instant re-draw, call `set_status_text()`
    pub(crate) fn set_status_idle_text(&mut self, msg: &str) { self.status_idle_text = msg.to_owned(); }

    /// Layout the post bubbles on the screen.
    ///
    /// The challenge is to layout a sub-set of the posts on screen, ensuring that
    /// the selected-post is fully displayed, and to do something non-jarring as the
    /// user moves the selection up or down.
    ///
    /// That is, when the user clicks up then the currently selected post should go
    /// un-bold, and the post above should go bold, without movement - unless the newly
    /// selected post is partially or fully off-screen, in which case, the posts need
    /// to move down. There are three edge cases, when the first or last post is reached,
    /// or when the post is too big for the screen. And an additional challenge,
    /// that the only way to calculate the vertical height of a post is to lay it out.
    fn layout(&mut self) -> Result<(), Error> {
        if let Some(dialogue) = self.dialogue.as_mut() {
            log::info!("redrawing dialogue: {}", dialogue.title);

            // 1. Consistency check the layout range versus selected post.
            let search_required = if let Some(selected) = self.layout_selected {
                !self.layout_range.contains(&selected)
            } else {
                true
            };

            // 2. Adjust the displayable range.
            if search_required {
                let starting_at = if let Some(selected) = self.layout_selected {
                    if self.layout_range.len() > 0 {
                        self.layout_topdown = selected <= *self.layout_range.iter().min().unwrap_or(&0);
                        selected
                    } else {
                        // if no range is available, go from the bottom up, starting with the selected post
                        self.layout_topdown = false;
                        selected
                    }
                } else {
                    // no post selected, always layout from bottom up, starting at the most recent post
                    self.layout_topdown = false;
                    dialogue.post_last().unwrap_or(0)
                };
                let mut fwd_iter;
                let mut rev_iter;
                let search_window: &mut dyn Iterator<Item = _> = if self.layout_topdown {
                    // search from the selected post to all newer posts, top-to-down
                    fwd_iter = dialogue.posts_as_slice_mut()[starting_at..].iter_mut();
                    &mut fwd_iter
                } else {
                    // search from oldest post to selected post, bottom-to-top
                    if dialogue.posts_as_slice().len() > 0 {
                        rev_iter = dialogue.posts_as_slice_mut()[..=starting_at].iter_mut().rev();
                    } else {
                        // zero-length case we still have to return an empty iterator, but
                        // we can't have the range be inclusive and the code still work
                        rev_iter = dialogue.posts_as_slice_mut().iter_mut().rev();
                    }
                    &mut rev_iter
                };
                let mut total_height = 0;
                self.layout_range.clear();
                for (i, post) in search_window.enumerate() {
                    let body_height = if let Some(bb) = post.bounding_box {
                        bb.height()
                    } else {
                        // if the "natural height" has not been computed, do so now.
                        let mut layout_bubble = default_textview(post, false, &self.vp);
                        log::debug!("compute bounds on {}", layout_bubble);
                        if self.gam.bounds_compute_textview(&mut layout_bubble).is_ok() {
                            post.bounding_box = layout_bubble.bounds_computed;
                            match layout_bubble.bounds_computed {
                                Some(r) => r.height(),
                                None => {
                                    log::warn!(
                                        "Unexpected null bounds in computing textview heights, layout will be incorrect."
                                    );
                                    0
                                }
                            }
                        } else {
                            log::warn!(
                                "Unexpected error in computing textview heights, layout will be incorrect."
                            );
                            0
                        }
                    };
                    // the author-chrome header (a Bold line above the bubble)
                    // adds its own measured height; cached in header_box the
                    // way the body caches bounding_box
                    let header_height = if header_wanted(self.vp.bubbles, post.header()) {
                        if post.header_box.is_none() {
                            if let Some(mut header_tv) = header_textview(post, &self.vp) {
                                if self.gam.bounds_compute_textview(&mut header_tv).is_ok() {
                                    post.header_box = header_tv.bounds_computed;
                                }
                            }
                        }
                        post.header_box.map(|r| r.height()).unwrap_or(0)
                    } else {
                        0
                    };
                    // the send-time/mark footer (a Regular line below the
                    // bubble; ISSUES r6 row 20) is measured the same way,
                    // cached in footer_box
                    let footer_height = if footer_wanted(self.vp.bubbles, post.footer()) {
                        if post.footer_box.is_none() {
                            if let Some(mut footer_tv) = footer_textview(post, &self.vp) {
                                if self.gam.bounds_compute_textview(&mut footer_tv).is_ok() {
                                    post.footer_box = footer_tv.bounds_computed;
                                }
                            }
                        }
                        post.footer_box.map(|r| r.height()).unwrap_or(0)
                    } else {
                        0
                    };
                    let next_height = stacked_height(header_height, body_height, footer_height)
                        + self.vp.bubble_space as u32
                        + self.vp.bubble_margin.y as u32;
                    if total_height + next_height > self.vp.layout_screensize.y as u32 {
                        // `i` posts have been measured to fit; this one did not
                        self.layout_range = fitted_range(self.layout_topdown, starting_at, i);
                        break;
                    }
                    total_height += next_height;
                }
                if self.layout_range.len() == 0 {
                    // not enough elements to fill the entire screen. Just select everything from selected
                    // to the last possible message.
                    log::debug!("Not enough elements to fill the screen");
                    if self.layout_topdown {
                        self.layout_range = (starting_at..).collect();
                    } else {
                        if dialogue.posts_as_slice().len() > 0 {
                            self.layout_range = (0..=starting_at).rev().collect();
                        } else {
                            // "empty range" in case of no posts
                            self.layout_range = (0..0).rev().collect();
                        }
                    }
                }
            }
            assert!(
                dialogue.posts_as_slice().len() == 0 || self.layout_range.len() > 0,
                "Layout range should be set at this point."
            );

            // 3. clear the entire area, and re-draw the status bar
            self.gam
                .draw_rectangle(
                    self.vp.canvas,
                    Rectangle::new_with_style(
                        Point::new(0, 0),
                        self.vp.total_screensize,
                        DrawStyle {
                            fill_color: Some(PixelColor::Light),
                            stroke_color: None,
                            stroke_width: 0,
                        },
                    ),
                )
                .expect("can't clear canvas area");

            // 4. draw the text bubbles, in the order computed in step 2.
            let mut y = if self.layout_topdown {
                self.vp.status_height as isize + self.vp.bubble_margin.y
            } else {
                self.vp.status_height as isize + self.vp.layout_screensize.y - self.vp.bubble_margin.y
            };
            log::debug!(
                "Laying out with selected {:?} in range {:?}; topdown: {:?}",
                self.layout_selected,
                self.layout_range,
                self.layout_topdown
            );
            for &post_index in &self.layout_range {
                let post = match dialogue.post_get(post_index) {
                    Some(p) => p,
                    None => {
                        log::warn!(
                            "Expected post at index {}, returned nothing. Range {:?}, posts {:?}",
                            post_index,
                            self.layout_range,
                            dialogue.posts_as_slice()
                        );
                        continue;
                    }
                };
                let highlight =
                    if let Some(selected) = self.layout_selected { selected == post_index } else { false };
                // Stack header above the body and footer below it, in
                // absolute screen order, whichever way the overall list
                // is scrolling (ignore/notes-r5-author-identity.md
                // section 6.2, extended for the footer in ISSUES r6 row
                // 20). Whichever of header/footer is knowable from `y`
                // alone draws first; the other depends on the body's
                // *actual* drawn height and draws after the body:
                //   topdown:   header (at y) -> body -> footer (depends on body)
                //   bottom-up: footer (at y) -> body -> header (depends on body)
                let mut header_height: u32 = 0;
                let mut footer_height: u32 = 0;
                if self.layout_topdown {
                    if let Some(mut header_tv) = header_bubble(&self.vp, true, post, dialogue, y) {
                        self.gam.post_textview(&mut header_tv).expect("couldn't render header textview");
                        header_height = header_tv.bounds_computed.map(|r| r.height()).unwrap_or(0);
                    }
                } else {
                    if let Some(mut footer_tv) = footer_bubble(&self.vp, false, post, dialogue, y) {
                        self.gam.post_textview(&mut footer_tv).expect("couldn't render footer textview");
                        footer_height = footer_tv.bounds_computed.map(|r| r.height()).unwrap_or(0);
                    }
                }
                let body_anchor_y = if self.layout_topdown {
                    y + header_height as isize
                } else {
                    y - footer_height as isize
                };
                let mut bubble_tv =
                    bubble(&self.vp, self.layout_topdown, post, dialogue, highlight, body_anchor_y);
                self.gam.post_textview(&mut bubble_tv).expect("couldn't render bubble textview");
                // double check the actual bounds against expected bounds
                match bubble_tv.bounds_computed {
                    Some(actual_r) => {
                        // missing precomputed bounds are filled lazily; fall back to the drawn bounds
                        let expected_r = post.bounding_box.unwrap_or_else(|| {
                            log::warn!(
                                "post {} had no precomputed bounding box; using drawn bounds",
                                post_index
                            );
                            actual_r
                        });
                        if expected_r.height() != actual_r.height() {
                            log::warn!(
                                "Height mismatch of drawn versus pre-computed text (expected {}, got {}) for {}",
                                expected_r.height(),
                                actual_r.height(),
                                bubble_tv.to_str()
                            );
                        }
                        if self.layout_topdown {
                            if let Some(mut footer_tv) = footer_bubble(
                                &self.vp,
                                true,
                                post,
                                dialogue,
                                body_anchor_y + actual_r.height() as isize,
                            ) {
                                self.gam
                                    .post_textview(&mut footer_tv)
                                    .expect("couldn't render footer textview");
                                footer_height = footer_tv.bounds_computed.map(|r| r.height()).unwrap_or(0);
                            }
                        } else {
                            if let Some(mut header_tv) = header_bubble(
                                &self.vp,
                                false,
                                post,
                                dialogue,
                                body_anchor_y - actual_r.height() as isize,
                            ) {
                                self.gam
                                    .post_textview(&mut header_tv)
                                    .expect("couldn't render header textview");
                                header_height = header_tv.bounds_computed.map(|r| r.height()).unwrap_or(0);
                            }
                        }
                        // extend the expected-versus-actual check to the header and footer boxes
                        if let Some(expected_h) = post.header_box.map(|r| r.height()) {
                            if header_height != 0 && expected_h != header_height {
                                log::warn!(
                                    "Header height mismatch of drawn versus pre-computed (expected {}, got {})",
                                    expected_h,
                                    header_height
                                );
                            }
                        }
                        if let Some(expected_h) = post.footer_box.map(|r| r.height()) {
                            if footer_height != 0 && expected_h != footer_height {
                                log::warn!(
                                    "Footer height mismatch of drawn versus pre-computed (expected {}, got {})",
                                    expected_h,
                                    footer_height
                                );
                            }
                        }
                        let stacked = stacked_height(header_height, actual_r.height(), footer_height);
                        if self.layout_topdown {
                            y += stacked as isize;
                        } else {
                            y -= stacked as isize;
                        }
                        // sanity check the computations
                        if y > self.vp.layout_screensize.y + self.vp.status_height as isize
                            || y < self.vp.status_height as isize
                        {
                            log::error!(
                                "Computed range of elements sent to layout overflows at index {}",
                                post_index
                            );
                            // stop laying out to avoid text artifacts
                            break;
                        }
                        // add y-margin before the next iteration
                        if self.layout_topdown {
                            y += self.vp.bubble_space + self.vp.bubble_margin.y;
                        } else {
                            y -= self.vp.bubble_space + self.vp.bubble_margin.y;
                        }
                    }
                    _ => {
                        log::error!(
                            "No bounds computed for {}, this is a GAM or typesetter bug!",
                            bubble_tv.to_str()
                        );
                    }
                }
            }

            // 5. draw status bar on top of any post that happens to flow over the top...
            self.gam.post_textview(&mut self.status_tv).expect("couldn't render status bar");
            let status_border = Line::new(
                Point::new(0, self.vp.status_height as isize),
                Point::new(self.vp.total_screensize.x, self.vp.status_height as isize),
            );
            self.gam.draw_line(self.vp.canvas, status_border).expect("couldn't draw status lower border");

            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidData, "missing dialogue"))
        }
    }
}

/// True when the GAM's toggle-only ToggleMenuMode opcode must be sent to
/// reach `requested` from `tracked`. Pure so the idempotence contract is
/// testable without a live GAM: sending the toggle when the state already
/// matches would flip the IMEF back to prediction splicing.
pub(crate) fn imef_toggle_needed(tracked: bool, requested: bool) -> bool { tracked != requested }

/// True when a post's author-chrome header should be rendered: only on
/// bubble surfaces (a borderless full-width row has no border for the
/// header to sit outside of, and the border is the anti-spoof
/// property), and only when a non-empty header string exists. Pure so
/// the boot-log and overview exclusions are testable without a GAM.
pub(crate) fn header_wanted(bubbles: bool, header: Option<&str>) -> bool {
    bubbles && header.map_or(false, |h| !h.is_empty())
}

/// True when a post's send-time/mark footer should be rendered
/// (ISSUES r6 row 20): the same rule as `header_wanted`, mirrored
/// below the bubble instead of above it.
pub(crate) fn footer_wanted(bubbles: bool, footer: Option<&str>) -> bool {
    bubbles && footer.map_or(false, |f| !f.is_empty())
}

/// Total vertical extent of a header-plus-body-plus-footer stack.
/// Pure so the layout arithmetic is testable without a GAM.
pub(crate) fn stacked_height(h_header: u32, h_body: u32, h_footer: u32) -> u32 { h_header + h_body + h_footer }

/// The post indices to lay out: `fits` posts counting away from
/// `starting_at`, down the list when laying out top-down and up it when
/// laying out bottom-up. `fits` is how many posts were measured to fit,
/// so the post that overflowed the screen is excluded either way. It is
/// raised to one, because a post taller than the screen still has to be
/// drawn and an empty range sends the caller into its "not enough
/// elements" fallback, and capped at what the list holds. Pure so the
/// arithmetic is testable without a GAM.
pub(crate) fn fitted_range(topdown: bool, starting_at: usize, fits: usize) -> Vec<usize> {
    if topdown {
        (starting_at..starting_at + fits.max(1)).collect()
    } else {
        let fits = fits.clamp(1, starting_at + 1);
        (starting_at + 1 - fits..=starting_at).rev().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{fitted_range, footer_wanted, header_wanted, imef_toggle_needed, stacked_height};

    #[test]
    fn imef_toggle_only_on_state_change() {
        // default-off context, opt in: one toggle
        assert!(imef_toggle_needed(false, true));
        // already on, asked on again: no toggle (a second toggle would
        // re-enable the compose-line splice)
        assert!(!imef_toggle_needed(true, true));
        // explicit opt back out: one toggle
        assert!(imef_toggle_needed(true, false));
        // default-off, asked off: no toggle
        assert!(!imef_toggle_needed(false, false));
    }

    #[test]
    fn header_wanted_is_false_when_bubbles_are_off() {
        // the overview and the boot log (bubbles == false) must never
        // grow a header, whatever the post carries
        assert!(!header_wanted(false, Some("Alice (14:32)")));
    }

    #[test]
    fn header_wanted_is_false_without_a_header_string() {
        assert!(!header_wanted(true, None));
        assert!(!header_wanted(true, Some("")));
    }

    #[test]
    fn header_wanted_is_true_for_a_bubbled_post_with_a_header() {
        assert!(header_wanted(true, Some("Alice (14:32)")));
    }

    #[test]
    fn footer_wanted_is_false_when_bubbles_are_off() {
        // the overview and the boot log (bubbles == false) must never
        // grow a footer, whatever the post carries
        assert!(!footer_wanted(false, Some("14:32")));
    }

    #[test]
    fn footer_wanted_is_false_without_a_footer_string() {
        assert!(!footer_wanted(true, None));
        assert!(!footer_wanted(true, Some("")));
    }

    #[test]
    fn footer_wanted_is_true_for_a_bubbled_post_with_a_footer() {
        assert!(footer_wanted(true, Some("14:32")));
    }

    #[test]
    fn stacked_height_sums_header_body_and_footer() {
        assert_eq!(stacked_height(15, 32, 16), 63);
        assert_eq!(stacked_height(0, 32, 0), 32, "no header or footer adds nothing");
    }

    #[test]
    fn fitted_range_excludes_the_post_that_overflowed() {
        // the hosted log's case: newest post at 8, seven of them measured
        // to fit, so the range stops at 2 and never reaches 1
        assert_eq!(fitted_range(false, 8, 7), vec![8, 7, 6, 5, 4, 3, 2]);
        assert_eq!(fitted_range(true, 2, 7), vec![2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn fitted_range_keeps_a_post_taller_than_the_screen() {
        assert_eq!(fitted_range(false, 8, 0), vec![8]);
        assert_eq!(fitted_range(true, 8, 0), vec![8]);
    }

    #[test]
    fn fitted_range_stays_inside_the_post_list() {
        assert_eq!(fitted_range(false, 2, 9), vec![2, 1, 0]);
        assert_eq!(fitted_range(false, 0, 0), vec![0]);
    }
}
