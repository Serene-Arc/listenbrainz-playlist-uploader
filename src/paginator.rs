use url::Url;

pub struct ListenbrainzPaginator {
    base_url: String,
    current_position: usize,
    count_per_page: usize,
}

impl ListenbrainzPaginator {
    pub fn new(base_url: &str, start_position: usize, count_per_page: usize) -> Self {
        ListenbrainzPaginator {
            base_url: base_url.to_string(),
            current_position: start_position,
            count_per_page,
        }
    }
}

impl Iterator for ListenbrainzPaginator {
    type Item = Url;

    fn next(&mut self) -> Option<Self::Item> {
        let next = Url::parse_with_params(
            self.base_url.as_str(),
            [
                ("count", self.count_per_page.to_string()),
                ("offset", self.current_position.to_string()),
            ],
        )
        .expect("Could not construct URL");
        self.current_position += self.count_per_page;
        Some(next)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_paginator() {
        let mut test = ListenbrainzPaginator::new("https://www.example.com/", 0, 5);
        assert_eq!(
            "https://www.example.com/?count=5&offset=0",
            test.next().unwrap().as_str()
        );
        assert_eq!(
            "https://www.example.com/?count=5&offset=5",
            test.next().unwrap().as_str()
        );
        assert_eq!(
            "https://www.example.com/?count=5&offset=10",
            test.next().unwrap().as_str()
        );
    }
}
