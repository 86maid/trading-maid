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
        self.inner
            .get(self.inner.len().saturating_sub(1).saturating_sub(index))
            .unwrap_or(&Decimal::MAX)
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
/// assert!(series == 9);
/// assert!(series + 6 == 15);
/// assert!(series - 6 == 3);
/// assert!(series * 3 == 27);
/// assert!(series / 3 == 3);
/// assert!(series[0] == 9);
/// assert!(series[series.len() - 1] == 1);
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
        if index >= self.inner.len() {
            return &u64::MAX;
        }

        self.inner
            .get(self.inner.len() - 1 - index)
            .unwrap_or(&u64::MAX)
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
