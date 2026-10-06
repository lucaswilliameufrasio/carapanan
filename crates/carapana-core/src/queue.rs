//! Ordered in-memory queue transitions shared by session frontends.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkQueue<T> {
    items: Vec<T>,
}

impl<T> WorkQueue<T> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, item: T) {
        self.items.push(item);
    }

    pub fn push_front(&mut self, item: T) {
        self.items.insert(0, item);
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index)
    }

    pub fn first(&self) -> Option<&T> {
        self.items.first()
    }

    pub fn last(&self) -> Option<&T> {
        self.items.last()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Removes the next item for processing, preserving FIFO order.
    pub fn take_next(&mut self) -> Option<T> {
        if self.items.is_empty() {
            None
        } else {
            Some(self.items.remove(0))
        }
    }

    /// Removes a specific queued item, if the index is still valid.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        (index < self.items.len()).then(|| self.items.remove(index))
    }

    /// Moves an item one position toward either end and returns its new index.
    pub fn move_by(&mut self, index: usize, offset: isize) -> Option<usize> {
        if !matches!(offset, -1 | 1) {
            return None;
        }
        let target = index.checked_add_signed(offset)?;
        if index >= self.items.len() || target >= self.items.len() {
            return None;
        }
        self.items.swap(index, target);
        Some(target)
    }
}

impl<T> FromIterator<T> for WorkQueue<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

impl<T> std::ops::Index<usize> for WorkQueue<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.items[index]
    }
}

impl<T> std::ops::IndexMut<usize> for WorkQueue<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.items[index]
    }
}

#[cfg(test)]
mod tests {
    use super::WorkQueue;

    #[test]
    fn should_take_items_in_fifo_order() {
        let mut queue = WorkQueue::new();
        queue.push("first");
        queue.push("second");

        assert_eq!(queue.take_next(), Some("first"));
        assert_eq!(queue.take_next(), Some("second"));
        assert_eq!(queue.take_next(), None);
    }

    #[test]
    fn should_reorder_and_remove_only_valid_queue_items() {
        let mut queue = WorkQueue::new();
        queue.push("first");
        queue.push("second");
        queue.push("third");

        assert_eq!(queue.move_by(2, -1), Some(1));
        assert_eq!(
            queue.iter().copied().collect::<Vec<_>>(),
            ["first", "third", "second"]
        );
        assert_eq!(queue.move_by(0, -1), None);
        assert_eq!(queue.move_by(0, 2), None);
        assert_eq!(queue.move_by(1, 0), None);
        assert_eq!(queue.move_by(2, 1), None);
        assert_eq!(queue.remove(1), Some("third"));
        assert_eq!(queue.remove(8), None);
        assert_eq!(
            queue.iter().copied().collect::<Vec<_>>(),
            ["first", "second"]
        );
    }
}
