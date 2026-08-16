use enumset::EnumSetType;
use rkyv::{Archive, Deserialize, Serialize};

// shorthand for the function keys F1 - F4
pub const F1: char = '\u{0011}';
pub const F2: char = '\u{0012}';
pub const F3: char = '\u{0013}';
pub const F4: char = '\u{0014}';

// these are used to increment and decrement the selected post
pub const POST_SELECTED_NEXT: usize = usize::MAX - 0;
pub const POST_SELECTED_PREV: usize = usize::MAX - 1;

#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum ChatOp {
    // Save the Dialogue to pddb (ie after PostAdd, PostDelete)
    DialogueSave = 0,
    /// Set the current Dialogue to be displayed
    DialogueSet,
    /// change the Chat UI in/out of focus
    GamChangeFocus,
    /// a line of text has arrived
    GamLine,
    /// receive rawkeys from gam
    GamRawkeys,
    /// redraw our Chat UI
    GamRedraw,
    /// Show some user help
    Help,
    /// Add a new MenuItem to the App menu
    MenuAdd,
    /// Add a new Post to the Dialogue
    PostAdd,
    /// Delete a Post from the Dialogue
    PostDel,
    /// Find a Post by timestamp and Author
    PostFind,
    PostFlag,
    /// Set the flags on an Author of the current Dialogue
    AuthorFlagsSet,
    /// Set status bar text
    SetStatusText,
    /// Run or stop the busy animation.
    SetBusyAnimationState,
    /// Set the status idle text (to be shown when exiting all busy states)
    SetStatusIdleText,
    /// Set the four icontray slot labels
    SetIcontrayLabels,
    /// Opt in/out of the IMEF's menu mode for this app's context.
    /// In menu mode F1-F4 and the arrows come back as control strings
    /// on the input line (which the Chat UI drops) instead of being
    /// spliced into the compose line as predictions. Named to avoid
    /// the unrelated app-side Ui::set_menu_mode (left/right menus).
    SetImefMenuMode,
    /// Set the glyph style and framing of Dialogue posts
    SetPostStyle,
    /// Update just the state of the busy animation, if any. Internal opcode.
    /// Will skip the update if called too often.
    UpdateBusy,
    /// Force update the busy bar, without rate throttling. Internal opcode.
    UpdateBusyForced,
    /// exit the application
    Quit,
    /// Add a new Post carrying optional author chrome (a Bold line
    /// drawn above the bubble, outside its border). Opt-in: apps that
    /// never send this are unaffected. Appended after Quit so the
    /// existing opcode numbering is untouched.
    PostAddWithHeader,
}

#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub(crate) enum BusyAnimOp {
    Start,
    Pump,
}

#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum IconOp {
    PostMenu = 0,
    F2Op,
    F3Op,
    AppMenu,
}

pub const POST_TEXT_MAX: usize = 3072;

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct Find {
    pub author: String,
    pub timestamp: u64,
    pub key: Option<usize>, // the return post key if found.
}

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct Dialogue {
    pub dict: String,
    pub key: Option<String>,
}

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct AuthorFlags {
    pub author: String,
    pub flags: u16, // AuthorFlag bits (EnumSet::as_u16)
}

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct IcontrayLabels {
    pub labels: [String; 4],
}

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct PostStyle {
    pub style: u32, // GlyphStyle discriminant (blitstr2 implements From<usize>)
    pub bubbles: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct Post {
    pub dialogue_id: String,
    pub author: String,
    pub timestamp: u64,
    pub text: String,
    pub attach_url: Option<String>,
}

/// Transient IPC form of a Post that carries optional author chrome
/// (ISSUES r5 row 15) and, since ISSUES r6, an optional footer and a
/// centered-boxless-row flag (rows 20/21/22). A separate struct
/// rather than new fields on [`Post`], because existing apps
/// construct `Post` as a struct literal (apps/mtxchat/src/listen.rs)
/// and must keep compiling untouched; only apps that opt in via
/// `Chat::post_add_with_header` / `Chat::post_add_full` build one of
/// these. Extended in place rather than adding a second transient
/// struct and opcode, per the R5 precedent this struct itself set.
#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct PostWithHeader {
    pub dialogue_id: String,
    pub author: String,
    pub timestamp: u64,
    pub text: String,
    pub attach_url: Option<String>,
    /// Chrome drawn above the bubble in `GlyphStyle::Bold`, outside
    /// the bubble border. Never derived from `text`.
    pub header: Option<String>,
    /// Chrome drawn below the bubble in `GlyphStyle::Regular`, outside
    /// the bubble border, aligned to the same side as the bubble
    /// (ISSUES r6 row 20). Never derived from `text`.
    pub footer: Option<String>,
    /// True for a centered, borderless, box-free system row (a date
    /// separator or the message-request instruction bar; ISSUES r6
    /// rows 21/22) instead of an ordinary left/right bubble. Ignored
    /// together with `header`/`footer`, which a centered row does not
    /// carry.
    pub center: bool,
}

/// Events are sent to the Chat App when key things occur in the Chat UI
#[derive(Debug, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum Event {
    Focus,
    F1,     // F1 button click
    F2,     // F2 button click
    F3,     // you get the idea
    F4,     // guess
    Up,     // Up click
    Down,   // Down click
    Left,   // Left click
    Right,  // Right click
    Top,    // Top of post list reached
    Bottom, // Bottom of post list reached
    Key,    // keystroke
    Post,   // new user Post committed
    Menu,   // menu item clicked
}

#[derive(
    Archive, Serialize, Deserialize, Debug, num_derive::FromPrimitive, num_derive::ToPrimitive, EnumSetType,
)]
pub enum PostFlag {
    Deleted,
    Draft,
    Hidden,
}

#[derive(
    Archive, Serialize, Deserialize, Debug, num_derive::FromPrimitive, num_derive::ToPrimitive, EnumSetType,
)]
pub enum AuthorFlag {
    Bold,
    Hidden,
    Right,
}

#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct BusyMessage {
    pub busy_msg: String,
}
