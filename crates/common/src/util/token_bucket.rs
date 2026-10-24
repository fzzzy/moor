// Copyright (C) 2026 Ryan Daum <ryan.daum@gmail.com> This program is free
// software: you can redistribute it and/or modify it under the terms of the GNU
// Affero General Public License as published by the Free Software Foundation,
// version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
// details.
//
// You should have received a copy of the GNU Affero General Public License along
// with this program. If not, see <https://www.gnu.org/licenses/>.

//! Token-bucket rate limiting.

use std::time::Instant;

/// A token bucket: `rate` tokens per second, holding at most twice that, so a quiet sender may
/// burst briefly.
#[derive(Debug)]
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    /// `rate` tokens per second, starting full; 0 disables the limit.
    pub fn new(rate: u32, now: Instant) -> Self {
        let rate = f64::from(rate);
        Self {
            rate,
            burst: rate * 2.0,
            tokens: rate * 2.0,
            last: now,
        }
    }

    /// Take one token if available.
    pub fn allow(&mut self, now: Instant) -> bool {
        if self.rate == 0.0 {
            return true;
        }
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bursts_then_refills() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(50, start);
        let allowed = (0..150).filter(|_| bucket.allow(start)).count();
        assert_eq!(allowed, 100);
        assert!(!bucket.allow(start));
        // 100 ms refills five tokens.
        let later = start + Duration::from_millis(100);
        let allowed = (0..10).filter(|_| bucket.allow(later)).count();
        assert_eq!(allowed, 5);
        // The bucket never holds more than the burst.
        let much_later = start + Duration::from_secs(60);
        let allowed = (0..150).filter(|_| bucket.allow(much_later)).count();
        assert_eq!(allowed, 100);
    }

    #[test]
    fn zero_rate_is_unlimited() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(0, start);
        assert!((0..1000).all(|_| bucket.allow(start)));
    }
}
