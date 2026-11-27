use std::ops::Range;

struct Letter {
    key: u32,
    offset: usize,
    boundary: bool,
}

fn letters(units: &[u16]) -> impl Iterator<Item = Letter> + '_ {
    let mut previous = ' ';
    let mut offset = 0;
    char::decode_utf16(units.iter().copied()).map(move |decoded| {
        let (c, key) = match decoded {
            Ok(c) => (c, super::fold(c) as u32),
            Err(e) => (
                char::REPLACEMENT_CHARACTER,
                u32::from(e.unpaired_surrogate()),
            ),
        };
        let boundary = !previous.is_alphanumeric() || (previous.is_lowercase() && c.is_uppercase());
        let letter = Letter {
            key,
            offset,
            boundary,
        };
        offset += c.len_utf16();
        previous = c;
        letter
    })
}

pub(super) struct Query {
    keys: Vec<u32>,
    prefixes: Vec<usize>,
}

impl Query {
    pub(super) fn new(units: &[u16]) -> Self {
        let keys: Vec<_> = letters(units).map(|letter| letter.key).collect();
        let mut prefixes = vec![0; keys.len()];
        for i in 1..keys.len() {
            let mut n = prefixes[i - 1];
            while n > 0 && keys[i] != keys[n] {
                n = prefixes[n - 1];
            }
            prefixes[i] = n + usize::from(keys[i] == keys[n]);
        }
        Self { keys, prefixes }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    // KMP keeps overlapping literal occurrences, including one starting inside a looser match.
    fn literal(&self, text: &[Letter]) -> Option<usize> {
        let mut matched = 0;
        let mut first = None;
        for (index, letter) in text.iter().enumerate() {
            while matched > 0 && self.keys[matched] != letter.key {
                matched = self.prefixes[matched - 1];
            }
            matched += usize::from(self.keys[matched] == letter.key);
            if matched == self.keys.len() {
                let start = index + 1 - matched;
                if text[start].boundary {
                    return Some(start);
                }
                first.get_or_insert(start);
                matched = self.prefixes[matched - 1];
            }
        }
        first
    }
}

fn score(letters: &[Letter], hits: &[usize]) -> i64 {
    let first = hits[0];
    let last = hits[hits.len() - 1];
    let skipped = last - first + 1 - hits.len();
    let adjacent = hits
        .windows(2)
        .filter(|pair| pair[1] == pair[0] + 1)
        .count();
    let boundaries = hits.iter().filter(|&&i| letters[i].boundary).count();
    4 * adjacent as i64 + 2 * boundaries as i64 + i64::from(first == 0) - skipped as i64
}

pub(super) fn find(query: &Query, units: &[u16]) -> Option<(i64, Vec<Range<usize>>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    if query.keys.len() > units.len() {
        return None;
    }
    let letters: Vec<_> = letters(units).collect();
    let offset = |i: usize| letters.get(i).map_or(units.len(), |letter| letter.offset);
    if let Some(start) = query.literal(&letters) {
        let length = query.keys.len();
        // A gapped match scores at most six points per character.
        let rank = 6 * length as i64
            + i64::from(letters[start].boundary)
            + 2 * i64::from(start == 0)
            + 4 * i64::from(length == letters.len());
        return Some((
            rank,
            std::iter::once(offset(start)..offset(start + length)).collect(),
        ));
    }
    let query = &query.keys;
    let mut hits = Vec::new();
    let mut best_score = None;
    let mut best_hits = Vec::new();
    for (end, letter) in letters.iter().enumerate() {
        if letter.key != query[hits.len()] {
            continue;
        }
        hits.push(end);
        if hits.len() != query.len() {
            continue;
        }
        // The forward pass favours early word initials. Tightening the same window backwards
        // also finds adjacent letters near its end. Each pass visits a window only once.
        let forward_score = score(&letters, &hits);
        if best_score.is_none_or(|best| forward_score > best) {
            best_score = Some(forward_score);
            best_hits.clone_from(&hits);
        }
        let mut pos = end + 1;
        for (hit, &key) in hits.iter_mut().zip(query).rev() {
            pos -= 1;
            while letters[pos].key != key {
                pos -= 1;
            }
            *hit = pos;
        }
        let tight_score = score(&letters, &hits);
        if Some(tight_score) > best_score {
            best_score = Some(tight_score);
            best_hits.clone_from(&hits);
        }
        hits.clear();
    }
    let score = best_score?;
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for index in best_hits {
        let start = letters[index].offset;
        let end = offset(index + 1);
        if let Some(range) = ranges.last_mut().filter(|range| range.end == start) {
            range.end = end;
        } else {
            ranges.push(start..end);
        }
    }
    Some((score, ranges))
}
