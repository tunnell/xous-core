pub mod attach;
pub mod author;
pub mod post;

use core::slice::{Iter, IterMut};
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Read, Write};

use author::Author;
use gam::Gam;
use post::Post;
use rkyv::{Archive, Deserialize, Serialize};

use crate::ui::VisualProperties;
use crate::{default_textview, now};

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
    /// * `attach_url` - a url of an attachment (image for example)
    /// * `vp` - the visual properties of the system - so that we can pre-compute the size extents of the post
    pub fn post_add(
        &mut self,
        author: &str,
        timestamp: u64,
        text: &str,
        _attach_url: Option<&str>,
        vp: Option<(&VisualProperties, &Gam)>,
    ) -> Result<(), Error> {
        match self.author_id(author) {
            Some(author_id) => {
                let mut new = Post::new(
                    author_id, timestamp, text, None, // TODO implement
                );
                // compute the bounds for the post if visual properties are
                // specified. This must happen before the empty-dialogue
                // early return below, so the first post of a Dialogue
                // carries a bounding box like every later one instead
                // of entering with None.
                if let Some((vp, gam)) = vp {
                    let mut layout_bubble = default_textview(&new, false, vp);
                    log::debug!("Computing bounds on {:?}", layout_bubble);
                    if gam.bounds_compute_textview(&mut layout_bubble).is_ok() {
                        new.bounding_box = layout_bubble.bounds_computed;
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
                    // OR replace an existing post with matching timestamp & author.
                    // The scan must include the last index: a repost of the
                    // newest post (a send-status update) lands exactly there,
                    // and the old i..last bound silently dropped it.
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
                        // an equal-timestamp run reached the end without a
                        // matching author; the post still belongs in the list
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
            let last = self.posts.len() - 1;
            for n in i..last {
                if let Some(post) = self.posts.get(n) {
                    if post.timestamp() == timestamp {
                        if post.author_id() == *author_id {
                            return Some(n);
                        }
                    } else {
                        break;
                    }
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

    /// Serialize this Dialogue with rkyv and write the full archive to `writer`.
    ///
    /// A single `PddbKey::write` call transfers at most one IPC buffer (4072
    /// bytes, PDDB_BUF_DATA_LEN), so this must use `write_all`, which loops
    /// until every byte is accepted. Returns the archive length in bytes.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<usize, Error> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Dialogue archive is {} bytes, exceeds MAX_BYTES {}", bytes.len(), MAX_BYTES),
            ));
        }
        writer.write_all(&bytes)?;
        Ok(bytes.len())
    }

    /// Read a full archive from `reader` and deserialize it into a Dialogue.
    ///
    /// A single `PddbKey::read` call also transfers at most one IPC buffer,
    /// so this reads to the end of the key. The archive is validated with
    /// bytecheck before access: a truncated or corrupt record comes back as
    /// an error rather than undefined behaviour.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Dialogue, Error> {
        let mut bytes = Vec::<u8>::new();
        reader.take((MAX_BYTES + 2) as u64).read_to_end(&mut bytes)?;
        // rkyv archives must be accessed at their required alignment, which
        // Vec<u8> does not guarantee; AlignedVec does.
        let mut aligned = rkyv::util::AlignedVec::<16>::with_capacity(bytes.len());
        aligned.extend_from_slice(&bytes);
        let archive = rkyv::access::<ArchivedDialogue, rkyv::rancor::Error>(&aligned)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
        rkyv::deserialize::<Dialogue, rkyv::rancor::Error>(archive)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One PDDB IPC buffer holds 4072 bytes: PDDB_BUF_DATA_LEN in
    /// services/pddb/src/api.rs is pub(crate), so the value is restated here.
    const IPC_CAP: usize = 4072;

    /// Mimics PddbKey IO: a single read() or write() call transfers at most
    /// one IPC buffer (services/pddb/src/frontend/pddbkey.rs).
    struct ChunkedKey {
        data: Vec<u8>,
        pos: usize,
    }

    impl ChunkedKey {
        fn new() -> Self { Self { data: Vec::new(), pos: 0 } }

        fn rewind(&mut self) { self.pos = 0; }

        /// Models what dialogue_save does before writing: PddbKey has no
        /// truncate, so the key is deleted and recreated to drop stale bytes.
        fn recreate(&mut self) {
            self.data.clear();
            self.pos = 0;
        }
    }

    impl Write for ChunkedKey {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = buf.len().min(IPC_CAP);
            if self.pos + n > self.data.len() {
                self.data.resize(self.pos + n, 0);
            }
            self.data[self.pos..self.pos + n].copy_from_slice(&buf[..n]);
            self.pos += n;
            Ok(n)
        }

        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    impl Read for ChunkedKey {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let remaining = self.data.len().saturating_sub(self.pos);
            let n = remaining.min(buf.len()).min(IPC_CAP);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// Enough posts of known text that the archive exceeds one IPC buffer.
    fn big_dialogue() -> Dialogue {
        let mut dialogue = Dialogue::new("roundtrip");
        for i in 0..120u64 {
            let author = if i % 2 == 0 { "alice" } else { "bob" };
            let text = format!("post {:04} abcdefghijklmnopqrstuvwxyz0123456789", i);
            dialogue.post_add(author, 1_000_000 + i, &text, None, None).unwrap();
        }
        dialogue
    }

    #[test]
    fn dialogue_roundtrip_exceeds_one_ipc_buffer() {
        let dialogue = big_dialogue();
        let mut key = ChunkedKey::new();
        let written = dialogue.write_to(&mut key).unwrap();
        assert!(written > IPC_CAP, "archive must exceed one IPC buffer to prove the fix, got {}", written);
        assert_eq!(key.data.len(), written, "every archive byte must reach the key");

        key.rewind();
        let restored = Dialogue::read_from(&mut key).unwrap();
        assert_eq!(dialogue.posts_as_slice().len(), restored.posts_as_slice().len());
        for (a, b) in dialogue.posts().zip(restored.posts()) {
            assert_eq!(a.text().as_bytes(), b.text().as_bytes());
            assert_eq!(a.timestamp(), b.timestamp());
            assert_eq!(a.author_id(), b.author_id());
            assert_eq!(a.flags, b.flags);
        }
        // author names must survive too, or author_id equality proves nothing
        for post in restored.posts() {
            let original = dialogue.author(post.author_id()).map(|a| a.name.as_str());
            let read_back = restored.author(post.author_id()).map(|a| a.name.as_str());
            assert!(original.is_some());
            assert_eq!(original, read_back);
        }
    }

    #[test]
    fn shrinking_dialogue_leaves_no_stale_tail() {
        let big = big_dialogue();
        let mut small = Dialogue::new("shrunk");
        small.post_add("alice", 2_000_000, "just one short post", None, None).unwrap();

        // demonstrate the hazard: rewriting a shorter archive in place, as a
        // truncate-less key would, leaves the old tail bytes behind
        let mut key = ChunkedKey::new();
        let big_len = big.write_to(&mut key).unwrap();
        key.rewind();
        let small_len = small.write_to(&mut key).unwrap();
        assert!(small_len < big_len);
        assert_eq!(key.data.len(), big_len, "in-place rewrite keeps the stale tail");

        // reading that record back must fail validation (the rkyv root at
        // the tail is the old archive's), not deserialize as a Dialogue
        key.rewind();
        assert!(
            Dialogue::read_from(&mut key).is_err(),
            "stale-tail record must be rejected, not deserialized"
        );

        // the save path deletes and recreates the key instead
        key.recreate();
        let written = small.write_to(&mut key).unwrap();
        assert_eq!(key.data.len(), written, "no stale tail may survive a shrinking rewrite");
        key.rewind();
        let restored = Dialogue::read_from(&mut key).unwrap();
        assert_eq!(restored.posts_as_slice().len(), small.posts_as_slice().len());
        assert_eq!(restored.posts_as_slice()[0].text(), small.posts_as_slice()[0].text());
    }

    #[test]
    fn truncated_archive_is_an_error_not_ub() {
        let dialogue = big_dialogue();
        let mut key = ChunkedKey::new();
        dialogue.write_to(&mut key).unwrap();
        // a pre-fix save wrote at most one IPC buffer and dropped the rest
        key.data.truncate(IPC_CAP);
        key.rewind();
        assert!(Dialogue::read_from(&mut key).is_err(), "truncated archive must be a handled error");
    }
}
