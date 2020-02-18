//! Support for Hypermedia Application Language concepts

use std::collections::HashMap;

use rpki::uri;

#[derive(Deserialize, Serialize)]
pub struct CollectionLinks(HashMap<String, HashMap<String, uri::Https>>);

impl CollectionLinks {
    pub fn new(base: uri::Https, current_page: usize, nr_pages: usize) -> Self {
        let mut res = HashMap::new();

        let self_uri = if current_page > 0 {
            base.clone()
        } else {
            Self::page_uri(&base, current_page)
        };
        res.insert("self".to_string(), Self::href_map(self_uri));

        if nr_pages > 1 {
            let last_page = nr_pages - 1;

            if current_page > 0 {
                res.insert("first".to_string(), Self::href_map(base.clone()));
            }

            if current_page == 1 {
                res.insert("prev".to_string(), Self::href_map(base.clone()));
            }

            if current_page > 1 {
                res.insert(
                    "prev".to_string(),
                    Self::href_map(Self::page_uri(&base, current_page - 1)),
                );
            }

            if current_page + 1 <= last_page {
                res.insert(
                    "next".to_string(),
                    Self::href_map(Self::page_uri(&base, current_page + 1)),
                );
            }

            if current_page < last_page {
                res.insert(
                    "last".to_string(),
                    Self::href_map(Self::page_uri(&base, last_page)),
                );
            }
        }

        CollectionLinks(res)
    }

    fn page_uri(base: &uri::Https, page: usize) -> uri::Https {
        let base_uri = base.as_str();
        let uri = if base_uri.contains('?') {
            format!("{}&page={}", base, page)
        } else {
            format!("{}?page={}", base, page)
        };
        uri::Https::from_string(uri).unwrap()
    }

    fn href_map(uri: uri::Https) -> HashMap<String, uri::Https> {
        let mut res = HashMap::new();
        res.insert("href".to_string(), uri);
        res
    }
}

#[derive(Deserialize, Serialize)]
pub struct Collection<T> {
    #[serde(rename(serialize = "_links", deserialize = "_links"))]
    links: CollectionLinks,
    count: usize,
    total: usize,
    #[serde(rename(serialize = "_embedded", deserialize = "_embedded"))]
    embedded: HashMap<String, Vec<T>>,
}

impl<T> Collection<T> {
    pub fn new(
        base: uri::Https,
        total: usize,
        offset: usize,
        page_size: usize,
        embedded_key: &str,
        embedded_items: Vec<T>,
    ) -> Self {
        let count = embedded_items.len();
        let current_page = offset / page_size;
        let nr_pages = Self::nr_pages(total, page_size);

        let links = CollectionLinks::new(base, current_page, nr_pages);
        let mut embedded = HashMap::new();
        embedded.insert(embedded_key.to_string(), embedded_items);

        Collection {
            links,
            count,
            total,
            embedded,
        }
    }

    fn nr_pages(total: usize, page_size: usize) -> usize {
        if total % page_size > 0 {
            total / page_size + 1
        } else {
            total / page_size
        }
    }
}

//============ Tests =========================================================

#[cfg(test)]
mod test {

    use super::*;
    use std::str::FromStr;

    #[derive(Deserialize, Serialize)]
    struct Item {
        name: String,
        number: usize,
    }

    impl Item {
        pub fn new(name: &str, number: usize) -> Self {
            Item {
                name: name.to_string(),
                number,
            }
        }
    }

    #[test]
    fn calculate_nr_pages() {
        assert_eq!(0, Collection::<String>::nr_pages(0, 10));
        assert_eq!(1, Collection::<String>::nr_pages(1, 10));
        assert_eq!(1, Collection::<String>::nr_pages(9, 10));
        assert_eq!(1, Collection::<String>::nr_pages(10, 10));
        assert_eq!(2, Collection::<String>::nr_pages(11, 10));
        assert_eq!(2, Collection::<String>::nr_pages(19, 10));
        assert_eq!(2, Collection::<String>::nr_pages(20, 10));
        assert_eq!(3, Collection::<String>::nr_pages(21, 10));
    }

    #[test]
    fn serialize_collection() {
        let all_items = vec![Item::new("a", 1), Item::new("b", 2), Item::new("c", 3)];
        let some_items = vec![Item::new("a", 1), Item::new("b", 2)];
        let base = uri::Https::from_str("https://localhost/path/to/items").unwrap();
        let collection = Collection::new(base, all_items.len(), 0, 2, "items", some_items);

        println!("{}", serde_json::to_string_pretty(&collection).unwrap());
    }
}
