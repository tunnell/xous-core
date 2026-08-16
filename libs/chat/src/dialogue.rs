pub mod attach;
pub mod author;
pub mod post;

use core::slice::{Iter, IterMut};
use std::collections::HashMap;
use std::io::{Error, ErrorKind};

use author::Author;
use enumset::EnumSet;
use gam::Gam;
use post::Post;
use rkyv::{Archive, Deserialize, Serialize};

use crate::ui::VisualProperties;
use crate::api::AuthorFlag;
use crate::{default_textview, footer_textview, header_textview, now};

// TODO do better than just allocate lots!
pub const MAX_BYTES: usize = 65536;

/// A Dialogue is a generic representation of a series of Posts
/// This might represent a room, group, or direct-message conversation
#[derive(Archive, Serialize, Deserialize, Debug)]
pub struct Dialogue {
    /// A title of the Dialogue
    pub title: String,
    /// A time ordered sequence of posts in the Dialogue
    posts: Vec<Post>,
    /// An index of unique Author id's (internal)
    authors: HashMap<u16, Author>,
    /// A lookup on Author names
    author_lookup: HashMap<String, u16>,
    /// The timestamp on the most recent Post
    last_timestamp: u64,
    /// The id assigned to the most recent new Author
    last_author_id: u16,
}

impl Dialogue {
    /// Creates a new Dialogue with a single Author.
    /// Author id=0 is assigned to the user of this Chat App.
    pub fn new(title: &str) -> Self {
        let first_author_id = 0;
        let author = Author::new("me");
        let mut authors = HashMap::new();
        authors.insert(first_author_id, author);
        Self {
            title: title.to_string(),
            posts: Vec::<Post>::new(),
            authors,
            author_lookup: HashMap::<String, u16>::new(),
            last_timestamp: now(),
            last_author_id: first_author_id + 1,
        }
    }

    /// Add a new Post to the Dialogue
    ///
    /// TODO protect against Dialogue::MAX_BYTES overflow
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
    /// * `header` - optional author chrome drawn Bold above the bubble, outside its border
    /// * `footer` - optional chrome drawn Regular below the bubble, outside its border (ISSUES r6 row 20)
    /// * `center` - true for a centered, borderless, box-free system row instead of a left/right bubble
    ///   (ISSUES r6 rows 21/22); ignored together with `header`/`footer`, which a centered row does not carry
    /// * `attach_url` - a url of an attachment (image for example)
    /// * `vp` - the visual properties of the system - so that we can pre-compute the size extents of the post
    pub fn post_add(
        &mut self,
        author: &str,
        timestamp: u64,
        text: &str,
        header: Option<&str>,
        footer: Option<&str>,
        center: bool,
        _attach_url: Option<&str>,
        vp: Option<(&VisualProperties, &Gam)>,
    ) -> Result<(), Error> {
        match self.author_id(author) {
            Some(author_id) => {
                let mut new = Post::new(
                    author_id, timestamp, text, header, footer, center,
                    None, // TODO implement attachments
                );
                // compute the bounds for the post if visual properties are specified. This must
                // precede the empty-dialogue early return so the first post also gets a bounding box.
                if let Some((vp, gam)) = vp {
                    let mut layout_bubble = default_textview(&new, false, vp);
                    log::debug!("Computing bounds on {:?}", layout_bubble);
                    if gam.bounds_compute_textview(&mut layout_bubble).is_ok() {
                        new.bounding_box = layout_bubble.bounds_computed;
                    }
                    // measure the header and footer chrome alongside the
                    // body, so the insert-time cache covers the whole
                    // stacked extent
                    if let Some(mut header_tv) = header_textview(&new, vp) {
                        if gam.bounds_compute_textview(&mut header_tv).is_ok() {
                            new.header_box = header_tv.bounds_computed;
                        }
                    }
                    if let Some(mut footer_tv) = footer_textview(&new, vp) {
                        if gam.bounds_compute_textview(&mut footer_tv).is_ok() {
                            new.footer_box = footer_tv.bounds_computed;
                        }
                    }
                }
                if self.posts.len() == 0 {
                    self.posts.push(new);
                    return Ok(());
                }
                let new_ts = new.timestamp();
                let first_ts = self.posts.first().map_or(0, |p| p.timestamp());
                let last_ts = self.posts.last().map_or(0, |p| p.timestamp());
                log::trace!("{:?}", new);

                if new_ts > last_ts {
                    log::info!("insert new post at end");
                    self.posts.push(new);
                } else if new_ts < first_ts {
                    log::info!("insert new post at start");
                    self.posts.insert(0, new);
                } else {
                    // insert a new post in the correct position
                    // OR replace an existing post with matching timestamp & author. The scan must
                    // include the last index: a repost of the newest post (a send-status update)
                    // lands exactly there.
                    let i = self.posts.partition_point(|p| p.timestamp() < new_ts);
                    let mut new = Some(new);
                    for n in i..self.posts.len() {
                        if self.posts[n].timestamp() == new_ts {
                            if self.posts[n].author_id() == author_id {
                                log::info!("replace matching post at {n}");
                                self.posts[n] = new.take().unwrap();
                                break;
                            }
                        } else {
                            log::info!("insert new post at {n}");
                            self.posts.insert(n, new.take().unwrap());
                            break;
                        }
                    }
                    if let Some(p) = new {
                        // an equal-timestamp run ended without an author match; the post still belongs
                        self.posts.push(p);
                    }
                }
                Ok(())
            }
            None => Err(Error::new(ErrorKind::Other, "max authors exceeeded")),
        }
    }

    /// Returns Some(index) of a matching Post by Author and Timestamp, or None
    ///
    /// # Arguments
    ///
    /// * `timestamp` - the Post timestamp criteria
    /// * `author` - the Post Author criteria
    pub fn post_find(&self, author: &str, timestamp: u64) -> Option<usize> {
        if let Some(author_id) = self.author_lookup.get(author) {
            let i = self.posts.partition_point(|p| p.timestamp() < timestamp);
            // the scan must run through the last index: the newest post is the common find target
            for (n, post) in self.posts.iter().enumerate().skip(i) {
                if post.timestamp() != timestamp {
                    break;
                }
                if post.author_id() == *author_id {
                    return Some(n);
                }
            }
        }
        None
    }

    /// Return Some<Post> by index in the Dialogue, or None.
    ///
    /// # Arguments
    ///
    /// * `index` - the index of the required Post
    pub fn post_get(&self, index: usize) -> Option<&Post> { self.posts.get(index) }

    /// Return the index of the most recent Post in the Dialogue
    pub fn post_last(&self) -> Option<usize> {
        if self.posts.len() == 0 { None } else { Some(self.posts.len() - 1) }
    }

    /// Return a slice of posts
    pub fn posts_as_slice(&self) -> &[Post] { &self.posts }

    pub fn posts_as_slice_mut(&mut self) -> &mut [Post] { &mut self.posts }

    /// Return an iterator over the Dialogue Posts (oldest first)
    pub fn posts(&self) -> Iter<Post> { return self.posts.iter(); }

    /// Return a mut iterator over the Dialogue Posts (oldest first)
    pub fn posts_mut(&mut self) -> IterMut<Post> { return self.posts.iter_mut(); }

    /// Return Some<Author> by id, or None.
    ///
    /// # Arguments
    ///
    /// * `id` - the index of the required Author
    pub fn author(&self, id: u16) -> Option<&Author> { self.authors.get(&id) }

    /// Set the flags on an Author by name (the Author is added if new)
    ///
    /// # Arguments
    ///
    /// * `name` - the (external) name of the Author
    /// * `flags` - the AuthorFlag set
    pub fn author_flags_set(&mut self, name: &str, flags: EnumSet<AuthorFlag>) {
        if let Some(id) = self.author_id(name) {
            if let Some(author) = self.authors.get_mut(&id) {
                author.flags_set(flags);
            }
        }
    }

    /// Return Some<author_id> by Author name, or None.
    ///
    /// # Arguments
    ///
    /// * `author` - the (external) name of the Author
    pub fn author_id(&mut self, author: &str) -> Option<u16> {
        match self.author_lookup.get(author) {
            Some(id) => Some(*id),
            None => {
                if let Some(id) = self.author_id_next() {
                    self.authors.insert(id, Author::new(author));
                    self.author_lookup.insert(author.to_string(), id);
                    Some(id)
                } else {
                    None
                }
            }
        }
    }

    /// Assign and Return Some<author_id>, or None
    fn author_id_next(&mut self) -> Option<u16> {
        if self.last_author_id < u16::max_value() {
            self.last_author_id += 1;
            Some(self.last_author_id)
        } else {
            None
        }
    }
}
