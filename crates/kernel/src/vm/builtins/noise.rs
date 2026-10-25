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

//! Deterministic procedural functions: seeded 2D simplex noise and a repeatable integer hash.
//!
//! Worlds are generated from these, so their results are a contract: the same inputs must
//! give the same outputs on every platform and in every later release. The tests pin exact
//! values; a change that moves them changes every world built on them.

/// One step of SplitMix64: a fast, well-mixed 64-bit hash.
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A hash of a seed and a sequence of integers.
#[must_use]
pub fn hash_ints(seed: i64, values: &[i64]) -> u64 {
    let mut h = splitmix64(seed as u64);
    for value in values {
        h = splitmix64(h ^ *value as u64);
    }
    h
}

/// An integer from 1 to `max` fixed by a seed and a sequence of integers, without the bias
/// a plain modulus would give.
#[must_use]
pub fn random_at(seed: i64, values: &[i64], max: i64) -> i64 {
    debug_assert!(max >= 1);
    let h = hash_ints(seed, values);
    ((u128::from(h) * max as u128) >> 64) as i64 + 1
}

/// The twelve gradient directions of classic simplex noise, projected onto the plane.
const GRADIENTS: [(f64, f64); 12] = [
    (1.0, 1.0),
    (-1.0, 1.0),
    (1.0, -1.0),
    (-1.0, -1.0),
    (1.0, 0.0),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (0.0, 1.0),
    (0.0, -1.0),
];

/// Seeded 2D simplex noise.
pub struct Simplex {
    perm: [u8; 512],
}

impl Simplex {
    /// The permutation table for `seed`: 0 to 255 shuffled by a seeded Fisher-Yates.
    #[must_use]
    pub fn new(seed: i64) -> Self {
        let mut table: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut state = seed as u64;
        for i in (1..256).rev() {
            state = splitmix64(state);
            let j = ((u128::from(state) * (i as u128 + 1)) >> 64) as usize;
            table.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for (i, slot) in perm.iter_mut().enumerate() {
            *slot = table[i & 255];
        }
        Self { perm }
    }

    /// Noise at one point, from about -1.0 to 1.0.
    #[must_use]
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        const F2: f64 = 0.366_025_403_784_438_6; // (sqrt(3) - 1) / 2
        const G2: f64 = 0.211_324_865_405_187_1; // (3 - sqrt(3)) / 6

        let s = (x + y) * F2;
        let i = (x + s).floor();
        let j = (y + s).floor();
        let t = (i + j) * G2;
        let x0 = x - (i - t);
        let y0 = y - (j - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - f64::from(i1) + G2;
        let y1 = y0 - f64::from(j1) + G2;
        let x2 = x0 - 1.0 + 2.0 * G2;
        let y2 = y0 - 1.0 + 2.0 * G2;

        let ii = (i as i64).rem_euclid(256) as usize;
        let jj = (j as i64).rem_euclid(256) as usize;
        let corner = |dx: usize, dy: usize, x: f64, y: f64| {
            let t = 0.5 - x * x - y * y;
            if t < 0.0 {
                return 0.0;
            }
            let g = self.perm[ii + dx + self.perm[jj + dy] as usize] as usize % 12;
            let (gx, gy) = GRADIENTS[g];
            let t2 = t * t;
            t2 * t2 * (gx * x + gy * y)
        };
        let n =
            corner(0, 0, x0, y0) + corner(i1 as usize, j1 as usize, x1, y1) + corner(1, 1, x2, y2);
        (70.0 * n).clamp(-1.0, 1.0)
    }
}

/// Fractal layering: each octave adds detail at `lacunarity` times the frequency and
/// `persistence` times the weight of the one before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Octaves {
    pub octaves: u32,
    pub frequency: f64,
    pub persistence: f64,
    pub lacunarity: f64,
}

impl Default for Octaves {
    fn default() -> Self {
        Self {
            octaves: 1,
            frequency: 1.0,
            persistence: 0.5,
            lacunarity: 2.0,
        }
    }
}

pub const MAX_OCTAVES: u32 = 16;

impl Octaves {
    /// Layered noise at one point, scaled back to about -1.0 to 1.0.
    #[must_use]
    pub fn sample(&self, noise: &Simplex, x: f64, y: f64) -> f64 {
        let mut frequency = self.frequency;
        let mut amplitude = 1.0;
        let mut total = 0.0;
        let mut weight = 0.0;
        for _ in 0..self.octaves {
            total += noise.sample(x * frequency, y * frequency) * amplitude;
            weight += amplitude;
            frequency *= self.lacunarity;
            amplitude *= self.persistence;
        }
        (total / weight).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_pinned() {
        // These values are a contract with every world generated from them.
        let noise = Simplex::new(42);
        let samples: Vec<String> = [(0.3, 0.7), (3.25, -7.75), (100.1, 200.2), (-12.0, 4.5)]
            .iter()
            .map(|(x, y)| format!("{:.12}", noise.sample(*x, *y)))
            .collect();
        assert_eq!(
            samples, PINNED_SAMPLES,
            "simplex noise changed; every generated world would change with it"
        );
        assert_eq!(hash_ints(42, &[1, 2, 3]), PINNED_HASH);
        assert_eq!(random_at(7, &[10, -20], 100), PINNED_RANDOM_AT);
    }

    const PINNED_SAMPLES: [&str; 4] = [
        "0.692706272943",
        "-0.497603792687",
        "-0.045393729935",
        "0.839341264861",
    ];
    const PINNED_HASH: u64 = 4_591_807_362_961_508_349;
    const PINNED_RANDOM_AT: i64 = 95;

    #[test]
    fn noise_stays_in_range_and_varies_smoothly() {
        let noise = Simplex::new(1);
        let mut previous = noise.sample(0.0, 0.3);
        let (mut low, mut high) = (f64::MAX, f64::MIN);
        for step in 1..10_000 {
            let x = f64::from(step) * 0.01;
            let value = noise.sample(x, 0.3);
            assert!((-1.0..=1.0).contains(&value));
            assert!(
                (value - previous).abs() < 0.1,
                "a step of 0.01 jumped at {x}"
            );
            previous = value;
            low = low.min(value);
            high = high.max(value);
        }
        assert!(
            low < -0.5 && high > 0.5,
            "noise should use most of its range"
        );
    }

    #[test]
    fn seeds_give_different_noise() {
        let a = Simplex::new(1);
        let b = Simplex::new(2);
        let differ = (0..100)
            .filter(|i| {
                let x = f64::from(*i) * 0.37;
                (a.sample(x, 1.1) - b.sample(x, 1.1)).abs() > 1e-9
            })
            .count();
        assert!(differ > 90);
    }

    #[test]
    fn octaves_add_detail_without_leaving_the_range() {
        let noise = Simplex::new(9);
        let layered = Octaves {
            octaves: 6,
            frequency: 0.05,
            ..Octaves::default()
        };
        for i in 0..1000 {
            let x = f64::from(i) * 1.7;
            let value = layered.sample(&noise, x, -x);
            assert!((-1.0..=1.0).contains(&value));
        }
        let one = Octaves::default();
        assert_eq!(one.sample(&noise, 3.3, 4.4), noise.sample(3.3, 4.4));
    }

    #[test]
    fn random_at_is_repeatable_and_in_range() {
        assert_eq!(random_at(5, &[1, 2], 6), random_at(5, &[1, 2], 6));
        let mut seen = [false; 6];
        for x in 0..600 {
            let value = random_at(5, &[x, 2], 6);
            assert!((1..=6).contains(&value));
            seen[(value - 1) as usize] = true;
        }
        assert!(seen.iter().all(|s| *s), "every face should come up");
        assert_ne!(
            hash_ints(5, &[1, 2]),
            hash_ints(5, &[2, 1]),
            "order matters"
        );
        assert_eq!(random_at(5, &[1], i64::MAX).signum(), 1);
    }
}
