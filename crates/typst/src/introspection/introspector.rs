use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use std::num::NonZeroUsize;

use comemo::Prehashed;
use ecow::{eco_format, EcoVec};
use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::diag::{bail, StrResult};
use crate::foundations::{Content, Label, Repr, Selector};
use crate::introspection::{Location, Meta};
use crate::layout::{Frame, FrameItem, Point, Position, Transform};
use crate::model::Numbering;
use crate::util::NonZeroExt;

/// Can be queried for elements and their positions.
#[derive(Clone)]
pub struct Introspector {
    /// The number of pages in the document.
    pages: usize,
    /// All introspectable elements.
    elems: IndexMap<Location, (Prehashed<Content>, Position)>,
    /// The page numberings, indexed by page number minus 1.
    page_numberings: Vec<Option<Numbering>>,
    /// Caches queries done on the introspector. This is important because
    /// even if all top-level queries are distinct, they often have shared
    /// subqueries. Example: Individual counter queries with `before` that
    /// all depend on a global counter query.
    queries: RefCell<HashMap<u128, EcoVec<Prehashed<Content>>>>,
    /// The label cache, maps labels to their indices in the element list.
    /// We use a smallvec such that if the label is unique, we don't need
    /// to allocate.
    label_cache: HashMap<Label, SmallVec<[usize; 1]>>,
}

impl Introspector {
    /// Create a new introspector.
    pub fn new(frames: &[Frame]) -> Self {
        Self::with_capacity(frames, 0)
    }

    /// Create a new introspector with a given capacity.
    #[tracing::instrument(skip(frames))]
    pub fn with_capacity(frames: &[Frame], capacity: usize) -> Self {
        let mut introspector = Self {
            pages: frames.len(),
            elems: IndexMap::with_capacity(capacity),
            page_numberings: Vec::with_capacity(capacity),
            queries: RefCell::default(),
            label_cache: HashMap::with_capacity(capacity),
        };
        for (i, frame) in frames.iter().enumerate() {
            let page = NonZeroUsize::new(1 + i).unwrap();
            introspector.extract(frame, page, Transform::identity());
        }
        introspector
    }

    /// Extract metadata from a frame.
    #[tracing::instrument(skip_all)]
    fn extract(&mut self, frame: &Frame, page: NonZeroUsize, ts: Transform) {
        for (pos, item) in frame.items() {
            match item {
                FrameItem::Group(group) => {
                    let ts = ts
                        .pre_concat(Transform::translate(pos.x, pos.y))
                        .pre_concat(group.transform);
                    self.extract(&group.frame, page, ts);
                }
                FrameItem::Meta(Meta::Elem(content), _)
                    if !self.elems.contains_key(&content.location().unwrap()) =>
                {
                    let pos = pos.transform(ts);
                    let content = Prehashed::new(content.clone());
                    let ret = self.elems.insert(
                        content.location().unwrap(),
                        (content.clone(), Position { page, point: pos }),
                    );
                    assert!(ret.is_none(), "duplicate locations");

                    // Build the label cache.
                    if let Some(label) = content.label() {
                        self.label_cache
                            .entry(label)
                            .or_insert_with(SmallVec::new)
                            .push(self.elems.len() - 1);
                    }
                }
                FrameItem::Meta(Meta::PageNumbering(numbering), _) => {
                    self.page_numberings.push(numbering.clone());
                }
                _ => {}
            }
        }
    }

    /// Iterate over all locatable elements.
    pub fn all(&self) -> impl Iterator<Item = &Prehashed<Content>> + '_ {
        self.elems.values().map(|(c, _)| c)
    }

    /// Get an element by its location.
    fn get(&self, location: &Location) -> Option<&Prehashed<Content>> {
        self.elems.get(location).map(|(elem, _)| elem)
    }

    /// Get the number of elements.
    pub fn len(&self) -> usize {
        self.elems.len()
    }

    /// Get the index of this element among all.
    fn index(&self, elem: &Content) -> usize {
        self.elems
            .get_index_of(&elem.location().unwrap())
            .unwrap_or(usize::MAX)
    }

    /// Perform a binary search for `elem` among the `list`.
    fn binary_search(
        &self,
        list: &[Prehashed<Content>],
        elem: &Content,
    ) -> Result<usize, usize> {
        list.binary_search_by_key(&self.index(elem), |elem| self.index(elem))
    }
}

#[comemo::track]
impl Introspector {
    /// Query for all matching elements.
    pub fn query(&self, selector: &Selector) -> EcoVec<Prehashed<Content>> {
        let hash = crate::util::hash128(selector);
        if let Some(output) = self.queries.borrow().get(&hash) {
            return output.clone();
        }

        let output = match selector {
            Selector::Label(label) => self
                .label_cache
                .get(label)
                .map(|indices| {
                    indices.iter().map(|&index| self.elems[index].0.clone()).collect()
                })
                .unwrap_or_default(),
            Selector::Elem(..) | Selector::Regex(_) | Selector::Can(_) => {
                self.all().filter(|elem| selector.matches(elem)).cloned().collect()
            }
            Selector::Location(location) => {
                self.get(location).cloned().into_iter().collect()
            }
            Selector::Before { selector, end, inclusive } => {
                let mut list = self.query(selector);
                if let Some(end) = self.query_first(end) {
                    // Determine which elements are before `end`.
                    let split = match self.binary_search(&list, &end) {
                        // Element itself is contained.
                        Ok(i) => i + *inclusive as usize,
                        // Element itself is not contained.
                        Err(i) => i,
                    };
                    list = list[..split].into();
                }
                list
            }
            Selector::After { selector, start, inclusive } => {
                let mut list = self.query(selector);
                if let Some(start) = self.query_first(start) {
                    // Determine which elements are after `start`.
                    let split = match self.binary_search(&list, &start) {
                        // Element itself is contained.
                        Ok(i) => i + !*inclusive as usize,
                        // Element itself is not contained.
                        Err(i) => i,
                    };
                    list = list[split..].into();
                }
                list
            }
            Selector::And(selectors) => {
                let mut results: Vec<_> =
                    selectors.iter().map(|sel| self.query(sel)).collect();

                // Extract the smallest result list and then keep only those
                // elements in the smallest list that are also in all other
                // lists.
                results
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, vec)| vec.len())
                    .map(|(i, _)| i)
                    .map(|i| results.swap_remove(i))
                    .iter()
                    .flatten()
                    .filter(|candidate| {
                        results
                            .iter()
                            .all(|other| self.binary_search(other, candidate).is_ok())
                    })
                    .cloned()
                    .collect()
            }
            Selector::Or(selectors) => selectors
                .iter()
                .flat_map(|sel| self.query(sel))
                .map(|elem| self.index(&elem))
                .collect::<BTreeSet<usize>>()
                .into_iter()
                .map(|index| self.elems[index].0.clone())
                .collect(),
        };

        self.queries.borrow_mut().insert(hash, output.clone());
        output
    }

    /// Query for the first element that matches the selector.
    pub fn query_first(&self, selector: &Selector) -> Option<Prehashed<Content>> {
        match selector {
            Selector::Location(location) => self.get(location).cloned(),
            _ => self.query(selector).first().cloned(),
        }
    }

    /// Query for a unique element with the label.
    pub fn query_label(&self, label: Label) -> StrResult<&Prehashed<Content>> {
        let indices = self.label_cache.get(&label).ok_or_else(|| {
            eco_format!("label `{}` does not exist in the document", label.repr())
        })?;

        if indices.len() > 1 {
            bail!("label `{}` occurs multiple times in the document", label.repr());
        }

        Ok(&self.elems[indices[0]].0)
    }

    /// The total number pages.
    pub fn pages(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.pages).unwrap_or(NonZeroUsize::ONE)
    }

    /// Gets the page numbering for the given location, if any.
    pub fn page_numbering(&self, location: Location) -> Option<&Numbering> {
        let page = self.page(location);
        self.page_numberings
            .get(page.get() - 1)
            .and_then(|slot| slot.as_ref())
    }

    /// Find the page number for the given location.
    pub fn page(&self, location: Location) -> NonZeroUsize {
        self.position(location).page
    }

    /// Find the position for the given location.
    pub fn position(&self, location: Location) -> Position {
        self.elems
            .get(&location)
            .map(|(_, loc)| *loc)
            .unwrap_or(Position { page: NonZeroUsize::ONE, point: Point::zero() })
    }
}

impl Default for Introspector {
    fn default() -> Self {
        Self::new(&[])
    }
}
