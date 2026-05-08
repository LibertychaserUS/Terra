use terra_cli::binary_search;

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
        assert_eq!(binary_search(&arr, &0), None);
    }

    #[test]
    fn test_found_at_beginning() {
        let arr = [1, 2, 3, 4, 5];
        assert_eq!(binary_search(&arr, &1), Some(0));
    }

    #[test]
    fn test_found_in_middle() {
        let arr = [1, 2, 3, 4, 5];
        assert_eq!(binary_search(&arr, &3), Some(2));
    }

    #[test]
    fn test_found_at_end() {
        let arr = [1, 2, 3, 4, 5];
        assert_eq!(binary_search(&arr, &5), Some(4));
    }

    #[test]
    fn test_not_found_less_than_all() {
        let arr = [10, 20, 30];
        assert_eq!(binary_search(&arr, &5), None);
    }

    #[test]
    fn test_not_found_greater_than_all() {
        let arr = [10, 20, 30];
        assert_eq!(binary_search(&arr, &40), None);
    }

    #[test]
    fn test_not_found_between() {
        let arr = [1, 3, 5, 7, 9];
        assert_eq!(binary_search(&arr, &4), None);
    }

    #[test]
    fn test_string_slice() {
        let words = ["apple", "banana", "cherry"];
        assert_eq!(binary_search(&words, &"banana"), Some(1));
        assert_eq!(binary_search(&words, &"dragonfruit"), None);
    }

    #[test]
    fn test_large_slice() {
        // 100_000 elements: 0..100_000
        let arr: Vec<i32> = (0..100_000).collect();
        assert_eq!(binary_search(&arr, &50_000), Some(50_000));
        assert_eq!(binary_search(&arr, &99_999), Some(99_999));
        assert_eq!(binary_search(&arr, &100_000), None);
    }
}
