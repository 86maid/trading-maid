//! The newest-first `Series` and `TimeSeries` views used by indicators and the context.

use overload::overload;
use rust_decimal::Decimal;
use std::{
    cmp::Ordering,
    fmt::{Display, Formatter, Result},
    mem::transmute,
    ops::{
        self, Deref, Index, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive,
    },
    slice::SliceIndex,
};

/// A data series with reverse indexing.
///
/// # Examples
///
/// ```
/// use trading_maid::prelude::*;
///
/// let data = [dec!(1.0), dec!(2.0), dec!(3.0), dec!(4.0), dec!(5.0), dec!(6.0), dec!(7.0), dec!(8.0), dec!(9.0)];
/// let series = Series::new(&data);
///
/// assert!(series == 9);
/// assert!(series + 6 == 15.0);
/// assert!(series - 6 == 3.0);
/// assert!(series * 3 == 27.0);
/// assert!(series / 3 == 3.0);
/// assert!(series[0] == 9.0);
/// assert!(series[series.len() - 1] == 1.0);
/// assert!(series[2..=7][..] == [dec!(2.0), dec!(3.0), dec!(4.0), dec!(5.0), dec!(6.0), dec!(7.0)][..]);
/// assert!(series[..=7][..] == [dec!(2.0), dec!(3.0), dec!(4.0), dec!(5.0), dec!(6.0), dec!(7.0), dec!(8.0), dec!(9.0)][..]);
/// assert!(series[5..][..] == [dec!(1.0), dec!(2.0), dec!(3.0), dec!(4.0)][..]);
///
/// let collected: Vec<Decimal> = series.iter().copied().collect();
///
/// assert_eq!(collected, [9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0]);
/// ```
#[derive(Debug)]
pub struct Series {
    pub inner: [Decimal],
}

impl Series {
    pub fn new(value: &[Decimal]) -> &Self {
        unsafe { transmute(value) }
    }

    fn slice<T>(&self, index: T) -> &Series
    where
        T: SliceIndex<[Decimal], Output = [Decimal]>,
    {
        unsafe { transmute(transmute::<_, &[Decimal]>(self).get(index).unwrap_or(&[])) }
    }
}

impl<'a> IntoIterator for &'a Series {
    type Item = &'a Decimal;
    type IntoIter = std::iter::Rev<std::slice::Iter<'a, Decimal>>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter().rev()
    }
}

impl Series {
    pub fn iter(&self) -> std::iter::Rev<std::slice::Iter<'_, Decimal>> {
        self.inner.iter().rev()
    }
}

impl Deref for Series {
    type Target = [Decimal];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Display for &Series {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.write_fmt(format_args!("{}", self[0]))
    }
}

impl From<&Series> for Decimal {
    fn from(val: &Series) -> Self {
        val[0]
    }
}

impl Index<usize> for Series {
    type Output = Decimal;

    fn index(&self, index: usize) -> &Self::Output {
        // 下标 0 是最新一根，底层 `inner` 最旧在前，所以真实位置是 `len - 1 - index`。
        //
        // 这里刻意不用裸减法：空序列时 `len - 1` 会下溢，而"越界返回哨兵值"也不该
        // 依赖"`index >= 0` 恒成立"这种巧合。用 checked_sub 逐级退化，
        // 空序列 / 越界 / index 极大，一律落到 `Decimal::MAX`。
        //
        // 之所以不用 `saturating_sub`：`len.saturating_sub(1).saturating_sub(index)`
        // 在越界时会被压到 0，返回的是**最老那根**的真实价格——一个看起来完全合法、
        // 实际却错位的数值，会静默污染指标与预热判断。
        let position = self
            .inner
            .len()
            .checked_sub(1)
            .and_then(|last| last.checked_sub(index))
            .and_then(|i| self.inner.get(i));

        match position {
            Some(value) => value,
            None => &Decimal::MAX,
        }
    }
}

impl Index<Range<usize>> for Series {
    type Output = Series;

    fn index(&self, index: Range<usize>) -> &Self::Output {
        self.slice(
            self.inner.len().saturating_sub(index.end)
                ..self.inner.len().saturating_sub(index.start),
        )
    }
}

impl Index<RangeFrom<usize>> for Series {
    type Output = Series;

    fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
        self.slice(..self.inner.len().saturating_sub(index.start))
    }
}

impl Index<RangeTo<usize>> for Series {
    type Output = Series;

    fn index(&self, index: RangeTo<usize>) -> &Self::Output {
        self.slice(self.inner.len().saturating_sub(index.end)..)
    }
}

impl Index<RangeFull> for Series {
    type Output = Series;

    fn index(&self, index: RangeFull) -> &Self::Output {
        self.slice(index)
    }
}

impl Index<RangeInclusive<usize>> for Series {
    type Output = Series;

    fn index(&self, index: RangeInclusive<usize>) -> &Self::Output {
        self.slice(
            self.inner
                .len()
                .saturating_sub(*index.end())
                .saturating_sub(1)..self.inner.len().saturating_sub(*index.start()),
        )
    }
}

impl Index<RangeToInclusive<usize>> for Series {
    type Output = Series;

    fn index(&self, index: RangeToInclusive<usize>) -> &Self::Output {
        self.slice(self.inner.len().saturating_sub(1).saturating_sub(index.end)..)
    }
}

overload!((a: &Series) + (b: &Series) -> Decimal { a[0] + b[0] });
overload!((a: &Series) - (b: &Series) -> Decimal { a[0] - b[0] });
overload!((a: &Series) * (b: &Series) -> Decimal { a[0] * b[0] });
overload!((a: &Series) / (b: &Series) -> Decimal { a[0] / b[0] });
overload!((a: &Series) % (b: &Series) -> Decimal { a[0] % b[0] });
overload!(- (a: &Series) -> Decimal { -a[0] });

macro_rules! overload_all_op_right {
    ($a:ty, $b:ty) => {
      overload!((a: $a) + (b: $b) -> Decimal { a[0] + Decimal::try_from(b).unwrap() });
      overload!((a: $a) - (b: $b) -> Decimal { a[0] - Decimal::try_from(b).unwrap() });
      overload!((a: $a) * (b: $b) -> Decimal { a[0] * Decimal::try_from(b).unwrap() });
      overload!((a: $a) / (b: $b) -> Decimal { a[0] / Decimal::try_from(b).unwrap() });
      overload!((a: $a) % (b: $b) -> Decimal { a[0] % Decimal::try_from(b).unwrap() });
    };
}

macro_rules! overload_all_op_left {
    ($a:ty, $b:ty) => {
        overload!((a: $a) + (b: $b) -> Decimal { Decimal::try_from(a).unwrap() + b[0] });
        overload!((a: $a) - (b: $b) -> Decimal { Decimal::try_from(a).unwrap() - b[0] });
        overload!((a: $a) * (b: $b) -> Decimal { Decimal::try_from(a).unwrap() * b[0] });
        overload!((a: $a) / (b: $b) -> Decimal { Decimal::try_from(a).unwrap() / b[0] });
        overload!((a: $a) % (b: $b) -> Decimal { Decimal::try_from(a).unwrap() % b[0] });
    };
}

overload_all_op_right!(&Series, &str);
overload_all_op_right!(&Series, String);
overload_all_op_right!(&Series, isize);
overload_all_op_right!(&Series, i8);
overload_all_op_right!(&Series, i16);
overload_all_op_right!(&Series, i32);
overload_all_op_right!(&Series, i64);
overload_all_op_right!(&Series, i128);
overload_all_op_right!(&Series, usize);
overload_all_op_right!(&Series, u8);
overload_all_op_right!(&Series, u16);
overload_all_op_right!(&Series, u32);
overload_all_op_right!(&Series, u64);
overload_all_op_right!(&Series, u128);
overload_all_op_right!(&Series, f32);
overload_all_op_right!(&Series, f64);
overload_all_op_right!(&Series, Decimal);

overload!((a: &str) + (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() + b[0] });
overload!((a: &str) - (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() - b[0] });
overload!((a: &str) * (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() * b[0] });
overload!((a: &str) / (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() / b[0] });
overload!((a: &str) % (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() % b[0] });
overload!((a: String) + (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() + b[0] });
overload!((a: String) - (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() - b[0] });
overload!((a: String) * (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() * b[0] });
overload!((a: String) / (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() / b[0] });
overload!((a: String) % (b: &Series) -> Decimal { Decimal::try_from(a).unwrap() % b[0] });

overload_all_op_left!(isize, &Series);
overload_all_op_left!(i8, &Series);
overload_all_op_left!(i16, &Series);
overload_all_op_left!(i32, &Series);
overload_all_op_left!(i64, &Series);
overload_all_op_left!(i128, &Series);
overload_all_op_left!(usize, &Series);
overload_all_op_left!(u8, &Series);
overload_all_op_left!(u16, &Series);
overload_all_op_left!(u32, &Series);
overload_all_op_left!(u64, &Series);
overload_all_op_left!(u128, &Series);
overload_all_op_left!(f32, &Series);
overload_all_op_left!(f64, &Series);
overload_all_op_left!(Decimal, &Series);

macro_rules! impl_decimal_eq {
    ($type:ty) => {
        impl PartialEq<$type> for &Series {
            #[track_caller]
            fn eq(&self, other: &$type) -> bool {
                self[0] == Decimal::try_from(*other).unwrap()
            }
        }

        impl PartialEq<&Series> for $type {
            #[track_caller]
            fn eq(&self, other: &&Series) -> bool {
                other[0] == *self
            }
        }
    };
}

impl PartialEq<str> for &Series {
    #[track_caller]
    fn eq(&self, other: &str) -> bool {
        self[0] == other
    }
}

impl PartialEq<&str> for &Series {
    #[track_caller]
    fn eq(&self, other: &&str) -> bool {
        self[0] == *other
    }
}

impl PartialEq<String> for &Series {
    #[track_caller]
    fn eq(&self, other: &String) -> bool {
        self[0] == other
    }
}

impl PartialEq<&String> for &Series {
    #[track_caller]
    fn eq(&self, other: &&String) -> bool {
        self[0] == *other
    }
}

impl<const N: usize> PartialEq<[Decimal; N]> for Series {
    #[track_caller]
    fn eq(&self, other: &[Decimal; N]) -> bool {
        self.inner == *other
    }
}

impl PartialEq<[Decimal]> for Series {
    #[track_caller]
    fn eq(&self, other: &[Decimal]) -> bool {
        self.inner == *other
    }
}

impl<const N: usize> PartialEq<Series> for [Decimal; N] {
    #[track_caller]
    fn eq(&self, other: &Series) -> bool {
        other.inner == *self
    }
}

impl PartialEq<Series> for [Decimal] {
    #[track_caller]
    fn eq(&self, other: &Series) -> bool {
        other.inner == *self
    }
}

impl PartialEq<&Series> for str {
    #[track_caller]
    fn eq(&self, other: &&Series) -> bool {
        *other == *self
    }
}

impl PartialEq<&Series> for &str {
    #[track_caller]
    fn eq(&self, other: &&Series) -> bool {
        *other == *self
    }
}

impl PartialEq<&Series> for String {
    #[track_caller]
    fn eq(&self, other: &&Series) -> bool {
        *other == *self
    }
}

impl PartialEq<&Series> for &String {
    #[track_caller]
    fn eq(&self, other: &&Series) -> bool {
        *other == *self
    }
}

impl PartialEq<&Series> for &Series {
    #[track_caller]
    fn eq(&self, other: &&Series) -> bool {
        other[0] == self[0]
    }
}

impl_decimal_eq!(isize);
impl_decimal_eq!(i8);
impl_decimal_eq!(i16);
impl_decimal_eq!(i32);
impl_decimal_eq!(i64);
impl_decimal_eq!(i128);
impl_decimal_eq!(usize);
impl_decimal_eq!(u8);
impl_decimal_eq!(u16);
impl_decimal_eq!(u32);
impl_decimal_eq!(u64);
impl_decimal_eq!(u128);
impl_decimal_eq!(f32);
impl_decimal_eq!(f64);
impl_decimal_eq!(Decimal);

macro_rules! impl_decimal_ord {
    ($type:ty) => {
        impl PartialOrd<$type> for &Series {
            fn partial_cmp(&self, other: &$type) -> Option<Ordering> {
                self[0].partial_cmp(&Decimal::try_from(*other).ok()?)
            }
        }

        impl PartialOrd<&Series> for $type {
            fn partial_cmp(&self, other: &&Series) -> Option<Ordering> {
                Decimal::try_from(*self).ok()?.partial_cmp(&other[0])
            }
        }
    };
}

impl PartialOrd<str> for &Series {
    fn partial_cmp(&self, other: &str) -> Option<core::cmp::Ordering> {
        self[0].partial_cmp(&Decimal::try_from(other).ok()?)
    }
}

impl PartialOrd<&str> for &Series {
    fn partial_cmp(&self, other: &&str) -> Option<core::cmp::Ordering> {
        self[0].partial_cmp(&Decimal::try_from(*other).ok()?)
    }
}

impl PartialOrd<String> for &Series {
    fn partial_cmp(&self, other: &String) -> Option<core::cmp::Ordering> {
        self[0].partial_cmp(&Decimal::try_from(other).ok()?)
    }
}

impl PartialOrd<&String> for &Series {
    fn partial_cmp(&self, other: &&String) -> Option<core::cmp::Ordering> {
        self[0].partial_cmp(&Decimal::try_from(*other).ok()?)
    }
}

impl PartialOrd<&Series> for str {
    fn partial_cmp(&self, other: &&Series) -> Option<core::cmp::Ordering> {
        other[0].partial_cmp(self)
    }
}

impl PartialOrd<&Series> for &str {
    fn partial_cmp(&self, other: &&Series) -> Option<core::cmp::Ordering> {
        other[0].partial_cmp(self)
    }
}

impl PartialOrd<&Series> for String {
    fn partial_cmp(&self, other: &&Series) -> Option<core::cmp::Ordering> {
        other[0].partial_cmp(self)
    }
}

impl PartialOrd<&Series> for &String {
    fn partial_cmp(&self, other: &&Series) -> Option<core::cmp::Ordering> {
        other[0].partial_cmp(self)
    }
}

impl PartialOrd<&Series> for &Series {
    fn partial_cmp(&self, other: &&Series) -> Option<core::cmp::Ordering> {
        other[0].partial_cmp(self)
    }
}

impl_decimal_ord!(isize);
impl_decimal_ord!(i8);
impl_decimal_ord!(i16);
impl_decimal_ord!(i32);
impl_decimal_ord!(i64);
impl_decimal_ord!(i128);
impl_decimal_ord!(usize);
impl_decimal_ord!(u8);
impl_decimal_ord!(u16);
impl_decimal_ord!(u32);
impl_decimal_ord!(u64);
impl_decimal_ord!(u128);
impl_decimal_ord!(f32);
impl_decimal_ord!(f64);
impl_decimal_ord!(Decimal);

/// A data series with reverse indexing.
///
/// # Examples
///
/// ```
/// use trading_maid::prelude::*;
///
/// let data = [1, 2, 3, 4, 5, 6, 7, 8, 9];
/// let series = TimeSeries::new(&data);
///
/// assert!(series == 9u64);
/// assert!(series + 6 == 15u64);
/// assert!(series - 6 == 3u64);
/// assert!(series * 3 == 27u64);
/// assert!(series / 3 == 3u64);
/// assert!(series[0] == 9u64);
/// assert!(series[series.len() - 1] == 1u64);
/// assert!(series[2..=7][..] == [2, 3, 4, 5, 6, 7][..]);
/// assert!(series[..=7][..] == [2, 3, 4, 5, 6, 7, 8, 9][..]);
/// assert!(series[5..][..] == [1, 2, 3, 4][..]);
///
/// let collected: Vec<u64> = series.iter().copied().collect();
///
/// assert_eq!(collected, [9, 8, 7, 6, 5, 4, 3, 2, 1]);
/// ```
#[derive(Debug)]
pub struct TimeSeries {
    pub inner: [u64],
}

impl TimeSeries {
    pub fn new(value: &[u64]) -> &Self {
        unsafe { transmute(value) }
    }

    fn slice<T>(&self, index: T) -> &TimeSeries
    where
        T: SliceIndex<[u64], Output = [u64]>,
    {
        unsafe { transmute(transmute::<_, &[u64]>(self).get(index).unwrap_or(&[])) }
    }
}

impl<'a> IntoIterator for &'a TimeSeries {
    type Item = &'a u64;
    type IntoIter = std::iter::Rev<std::slice::Iter<'a, u64>>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter().rev()
    }
}

impl TimeSeries {
    pub fn iter(&self) -> std::iter::Rev<std::slice::Iter<'_, u64>> {
        self.inner.iter().rev()
    }
}

impl Deref for TimeSeries {
    type Target = [u64];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Display for &TimeSeries {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.write_fmt(format_args!("{}", self[0]))
    }
}

impl From<&TimeSeries> for u64 {
    fn from(val: &TimeSeries) -> Self {
        val[0]
    }
}

impl Index<usize> for TimeSeries {
    type Output = u64;

    fn index(&self, index: usize) -> &Self::Output {
        // 与 `Series::index` 同一套规则：下标 0 是最新一根，越界返回 `u64::MAX`。
        // 同样避免裸减法，空序列时不会下溢。
        let position = self
            .inner
            .len()
            .checked_sub(1)
            .and_then(|last| last.checked_sub(index))
            .and_then(|i| self.inner.get(i));

        match position {
            Some(value) => value,
            None => &u64::MAX,
        }
    }
}

impl Index<Range<usize>> for TimeSeries {
    type Output = TimeSeries;

    fn index(&self, index: Range<usize>) -> &Self::Output {
        self.slice(
            self.inner.len().saturating_sub(index.end)
                ..self.inner.len().saturating_sub(index.start),
        )
    }
}

impl Index<RangeFrom<usize>> for TimeSeries {
    type Output = TimeSeries;

    fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
        self.slice(..self.inner.len().saturating_sub(index.start))
    }
}

impl Index<RangeTo<usize>> for TimeSeries {
    type Output = TimeSeries;

    fn index(&self, index: RangeTo<usize>) -> &Self::Output {
        self.slice(self.inner.len().saturating_sub(index.end)..)
    }
}

impl Index<RangeFull> for TimeSeries {
    type Output = TimeSeries;

    fn index(&self, index: RangeFull) -> &Self::Output {
        self.slice(index)
    }
}

impl Index<RangeInclusive<usize>> for TimeSeries {
    type Output = TimeSeries;

    fn index(&self, index: RangeInclusive<usize>) -> &Self::Output {
        self.slice(
            self.inner
                .len()
                .saturating_sub(*index.end())
                .saturating_sub(1)..self.inner.len().saturating_sub(*index.start()),
        )
    }
}

impl Index<RangeToInclusive<usize>> for TimeSeries {
    type Output = TimeSeries;

    fn index(&self, index: RangeToInclusive<usize>) -> &Self::Output {
        self.slice(self.inner.len().saturating_sub(1).saturating_sub(index.end)..)
    }
}

impl PartialEq<i32> for &TimeSeries {
    #[track_caller]
    fn eq(&self, other: &i32) -> bool {
        self.index(0) == &(*other as u64)
    }
}

impl PartialEq<u32> for &TimeSeries {
    #[track_caller]
    fn eq(&self, other: &u32) -> bool {
        self.index(0) == &(*other as u64)
    }
}

impl PartialEq<i64> for &TimeSeries {
    #[track_caller]
    fn eq(&self, other: &i64) -> bool {
        self.index(0) == &(*other as u64)
    }
}

impl PartialEq<u64> for &TimeSeries {
    #[track_caller]
    fn eq(&self, other: &u64) -> bool {
        self.index(0) == other
    }
}

impl<const N: usize> PartialEq<[u64; N]> for TimeSeries {
    fn eq(&self, other: &[u64; N]) -> bool {
        &self.inner == other
    }
}

impl PartialEq<[u64]> for TimeSeries {
    fn eq(&self, other: &[u64]) -> bool {
        &self.inner == other
    }
}

impl<const N: usize> PartialEq<TimeSeries> for [u64; N] {
    fn eq(&self, other: &TimeSeries) -> bool {
        other == self
    }
}

impl PartialEq<TimeSeries> for [u64] {
    fn eq(&self, other: &TimeSeries) -> bool {
        other == self
    }
}

impl PartialEq for &TimeSeries {
    fn eq(&self, other: &Self) -> bool {
        self.index(0) == other.index(0)
    }
}

impl PartialOrd<i64> for &TimeSeries {
    fn partial_cmp(&self, other: &i64) -> Option<Ordering> {
        self.index(0).partial_cmp(&(*other as u64))
    }
}

impl PartialOrd<u64> for &TimeSeries {
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        self.index(0).partial_cmp(other)
    }
}

impl PartialOrd for &TimeSeries {
    fn partial_cmp(&self, other: &&TimeSeries) -> Option<Ordering> {
        self.index(0).partial_cmp(other.index(0))
    }
}

macro_rules! ts_overload_all_op_right {
    ($a:ty, $b:ty) => {
      overload!((a: $a) + (b: $b) -> u64 { a[0] + b as u64});
      overload!((a: $a) - (b: $b) -> u64 { a[0] - b as u64});
      overload!((a: $a) * (b: $b) -> u64 { a[0] * b as u64});
      overload!((a: $a) / (b: $b) -> u64 { a[0] / b as u64});
      overload!((a: $a) % (b: $b) -> u64 { a[0] % b as u64});
    };
}

macro_rules! ts_overload_all_op_left {
    ($a:ty, $b:ty) => {
        overload!((a: $a) + (b: $b) -> u64 { a as u64 + b[0] });
        overload!((a: $a) - (b: $b) -> u64 { a as u64 - b[0] });
        overload!((a: $a) * (b: $b) -> u64 { a as u64 * b[0] });
        overload!((a: $a) / (b: $b) -> u64 { a as u64 / b[0] });
        overload!((a: $a) % (b: $b) -> u64 { a as u64 % b[0] });
    };
}

ts_overload_all_op_right!(&TimeSeries, isize);
ts_overload_all_op_right!(&TimeSeries, i8);
ts_overload_all_op_right!(&TimeSeries, i16);
ts_overload_all_op_right!(&TimeSeries, i32);
ts_overload_all_op_right!(&TimeSeries, i64);
ts_overload_all_op_right!(&TimeSeries, i128);
ts_overload_all_op_right!(&TimeSeries, usize);
ts_overload_all_op_right!(&TimeSeries, u8);
ts_overload_all_op_right!(&TimeSeries, u16);
ts_overload_all_op_right!(&TimeSeries, u32);
ts_overload_all_op_right!(&TimeSeries, u64);
ts_overload_all_op_right!(&TimeSeries, u128);
ts_overload_all_op_right!(&TimeSeries, f32);
ts_overload_all_op_right!(&TimeSeries, f64);

ts_overload_all_op_left!(isize, &TimeSeries);
ts_overload_all_op_left!(i8, &TimeSeries);
ts_overload_all_op_left!(i16, &TimeSeries);
ts_overload_all_op_left!(i32, &TimeSeries);
ts_overload_all_op_left!(i64, &TimeSeries);
ts_overload_all_op_left!(i128, &TimeSeries);
ts_overload_all_op_left!(usize, &TimeSeries);
ts_overload_all_op_left!(u8, &TimeSeries);
ts_overload_all_op_left!(u16, &TimeSeries);
ts_overload_all_op_left!(u32, &TimeSeries);
ts_overload_all_op_left!(u64, &TimeSeries);
ts_overload_all_op_left!(u128, &TimeSeries);
ts_overload_all_op_left!(f32, &TimeSeries);
ts_overload_all_op_left!(f64, &TimeSeries);

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // 验证 Series 按下标 0 = 最新读取，且越界返回哨兵值而不是最老那根的真实价格。
    #[test]
    fn index_reads_newest_first_and_out_of_bounds_returns_max() {
        let prices = [dec!(1), dec!(2), dec!(3)];
        let series = Series::new(&prices);

        assert_eq!(series[0], dec!(3));
        assert_eq!(series[1], dec!(2));
        assert_eq!(series[2], dec!(1));

        // 越界：返回 Decimal::MAX。
        // 旧实现用 saturating_sub 算下标，越界时被压到 0，返回的是 dec!(1) —— 最老那根，
        // 一个看起来完全合法、实际却错位的价格。
        assert_eq!(series[3], Decimal::MAX);
        assert_eq!(series[usize::MAX], Decimal::MAX);

        // 空序列：任何下标都返回哨兵值，且不得因 `len - 1` 下溢而 panic。
        // 这里连 `usize::MAX` 一起测，确保下标运算全程没有裸减法。
        let empty: [Decimal; 0] = [];
        let empty_series = Series::new(&empty);
        assert_eq!(empty_series[0], Decimal::MAX);
        assert_eq!(empty_series[1], Decimal::MAX);
        assert_eq!(empty_series[usize::MAX], Decimal::MAX);
    }

    // 验证 TimeSeries 的越界行为与 Series 一致（同一个哨兵值约定）。
    #[test]
    fn time_series_out_of_bounds_returns_max() {
        let times = [100u64, 200, 300];
        let series = TimeSeries::new(&times);

        assert_eq!(series[0], 300);
        assert_eq!(series[2], 100);
        assert_eq!(series[3], u64::MAX);
        assert_eq!(series[usize::MAX], u64::MAX);

        // 空序列同样不得下溢
        let empty: [u64; 0] = [];
        let empty_series = TimeSeries::new(&empty);
        assert_eq!(empty_series[0], u64::MAX);
        assert_eq!(empty_series[usize::MAX], u64::MAX);
    }
}
