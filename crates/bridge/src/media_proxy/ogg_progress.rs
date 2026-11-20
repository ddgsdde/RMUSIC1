//! Ogg granules give the buffered sample position even when the response has no Content-Length.
#[derive(Default)]
pub(super) struct Progress {
    header: Vec<u8>,
    body_left: usize,
    granule: u64,
    invalid: bool,
}

impl Progress {
    pub(super) fn push(&mut self, mut bytes: &[u8]) -> u64 {
        let mut complete = 0;
        while !bytes.is_empty() && !self.invalid {
            if self.body_left > 0 {
                let consumed = bytes.len().min(self.body_left);
                self.body_left -= consumed;
                bytes = &bytes[consumed..];
                if self.body_left == 0 && self.granule != u64::MAX {
                    complete = complete.max(self.granule);
                }
                continue;
            }
            let needed = if self.header.len() < 27 {
                27
            } else {
                27 + usize::from(self.header[26])
            };
            let consumed = bytes.len().min(needed - self.header.len());
            self.header.extend_from_slice(&bytes[..consumed]);
            bytes = &bytes[consumed..];
            if self.header.len() < 27 {
                break;
            }
            if &self.header[..5] != b"OggS\0" {
                self.invalid = true;
                break;
            }
            if self.header.len() != 27 + usize::from(self.header[26]) {
                continue;
            }

            self.granule = u64::from_le_bytes(self.header[6..14].try_into().unwrap());
            self.body_left = self.header[27..].iter().map(|&n| usize::from(n)).sum();
            self.header.clear();
            if self.body_left == 0 && self.granule != u64::MAX {
                complete = complete.max(self.granule);
            }
        }
        complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_only_granules_of_complete_pages_across_http_chunks() {
        let mut page = vec![0; 27];
        page[..4].copy_from_slice(b"OggS");
        page[6..14].copy_from_slice(&96_000u64.to_le_bytes());
        page[26] = 1;
        page.extend([4, 1, 2, 3, 4]);
        let mut progress = Progress::default();
        assert_eq!(progress.push(&page[..20]), 0);
        assert_eq!(progress.push(&page[20..31]), 0);
        assert_eq!(progress.push(&page[31..]), 96_000);

        page[6..14].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(progress.push(&page), 0);
    }
}
