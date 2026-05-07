/// Performs binary search on a sorted slice.
/// Returns `Some(index)` if the target is found, `None` otherwise.
///
/// # Examples
///
/// ```
/// use terra_cli::binary_search;
/// let arr = [1, 3, 5, 7, 9];
/// assert_eq!(binary_search(&arr, &5), Some(2));
/// assert_eq!(binary_search(&arr, &4), None);
/// ```
pub fn binary_search<T: Ord + std::cmp::PartialEq>(slice: &[T], target: &T) -> Option<usize> {
    let mut left = 0;
    let mut right = slice.len();

    while left < right {
        let mid = left + (right - left) / 2;

        match slice[mid].cmp(target) {
            std::cmp::Ordering::Equal => return Some(mid),
            std::cmp::Ordering::Less => left = mid + 1,
            std::cmp::Ordering::Greater => right = mid,
        }
    }

    None
}
