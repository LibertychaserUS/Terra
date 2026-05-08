/// Performs binary search on a sorted slice.
///
/// Returns the index of the key if found, or `None` if not present.
///
/// # Examples
///
/// ```
/// let arr = [1, 3, 5, 7, 9];
/// assert_eq!(terra_cli::binary_search(&arr, &5), Some(2));
/// assert_eq!(terra_cli::binary_search(&arr, &4), None);
/// ```
pub fn binary_search<T: Ord>(slice: &[T], key: &T) -> Option<usize> {
    let mut left = 0;
    let mut right = slice.len();

    while left < right {
        let mid = left + (right - left) / 2;
        match slice[mid].cmp(key) {
            std::cmp::Ordering::Equal => return Some(mid),
            std::cmp::Ordering::Less => left = mid + 1,
            std::cmp::Ordering::Greater => right = mid,
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_slice() {
        let arr: [i32; 0] = [];
        assert_eq!(binary_search(&arr, &42), None);
    }

    #[test]
    fn test_single_element_found() {
        let arr = [42];
        assert_eq!(binary_search(&arr, &42), Some(0));
    }

    #[test]
    fn test_single_element_not_found() {
        let arr = [42];
        assert_eq!(binary_search(&arr, &100), None);
    }

    #[test]
    fn test_multiple_elements_found() {
        let arr = [1, 3, 5, 7, 9];
        assert_eq!(binary_search(&arr, &1), Some(0));
        assert_eq!(binary_search(&arr, &5), Some(2));
        assert_eq!(binary_search(&arr, &9), Some(4));
    }

    #[test]
    fn test_multiple_elements_not_found() {
        let arr = [1, 3, 5, 7, 9];
        assert_eq!(binary_search(&arr, &0), None);
        assert_eq!(binary_search(&arr, &2), None);
        assert_eq!(binary_search(&arr, &4), None);
        assert_eq!(binary_search(&arr, &6), None);
        assert_eq!(binary_search(&arr, &8), None);
        assert_eq!(binary_search(&arr, &10), None);
    }

    #[test]
    fn test_string_slice() {
        let words = ["apple", "banana", "cherry", "date"];
        assert_eq!(binary_search(&words, &"banana"), Some(1));
        assert_eq!(binary_search(&words, &"grape"), None);
    }

    #[test]
    fn test_large_slice() {
        let arr: Vec<i32> = (0..1000).collect();
        assert_eq!(binary_search(&arr, &500), Some(500));
        assert_eq!(binary_search(&arr, &-1), None);
        assert_eq!(binary_search(&arr, &1000), None);
    }
}