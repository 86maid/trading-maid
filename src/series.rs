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
/// use trading_maid::series::Series;
/// use rust_decimal_macros::dec;
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

impl PartialEq<i32> for &Series {
    fn eq(&self, other: &i32) -> bool {
        self.index(0) == other
    }
}

impl PartialEq<u32> for &Series {
    fn eq(&self, other: &u32) -> bool {
        self.index(0) == other
    }
}

impl PartialEq<i64> for &Series {
    fn eq(&self, other: &i64) -> bool {
        self.index(0) == other
    }
}

impl PartialEq<u64> for &Series {
    fn eq(&self, other: &u64) -> bool {
        self.index(0) == other
    }
}

impl PartialEq<Decimal> for &Series {
    fn eq(&self, other: &Decimal) -> bool {
        self.index(0) == other
    }
}

impl PartialEq<[Decimal]> for Series {
    fn eq(&self, other: &[Decimal]) -> bool {
        &self.inner == other
    }
}

impl PartialEq for &Series {
    fn eq(&self, other: &Self) -> bool {
        self.index(0) == other.index(0)
    }
}

impl PartialOrd<i64> for &Series {
    fn partial_cmp(&self, other: &i64) -> Option<Ordering> {
        self.index(0).partial_cmp(other)
    }
}

impl PartialOrd<Decimal> for &Series {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        self.index(0).partial_cmp(other)
    }
}

impl PartialOrd<[Decimal]> for Series {
    fn partial_cmp(&self, other: &[Decimal]) -> Option<Ordering> {
        self.inner.partial_cmp(other)
    }
}

impl PartialOrd for &Series {
    fn partial_cmp(&self, other: &&Series) -> Option<Ordering> {
        self.index(0).partial_cmp(other.index(0))
    }
}

overload!((a: &Series) + (b: i64) -> Decimal { a[0] + b });

overload!((a: &Series) - (b: i64) -> Decimal { a[0] - b });

overload!((a: &Series) * (b: i64) -> Decimal { a[0] * b });

overload!((a: &Series) / (b: i64) -> Decimal { a[0] / b });

overload!((a: &Series) % (b: i64) -> Decimal { a[0] % b });

overload!((a: &Series) + (b: f64) -> Decimal { a[0] + b });

overload!((a: &Series) - (b: f64) -> Decimal { a[0] - b });

overload!((a: &Series) * (b: f64) -> Decimal { a[0] * b });

overload!((a: &Series) / (b: f64) -> Decimal { a[0] / b });

overload!((a: &Series) % (b: f64) -> Decimal { a[0] % b });

overload!((a: i64) + (b: &Series) -> Decimal { a + b[0] });

overload!((a: i64) - (b: &Series) -> Decimal { a - b[0] });

overload!((a: i64) * (b: &Series) -> Decimal { a * b[0] });

overload!((a: i64) / (b: &Series) -> Decimal { a / b[0] });

overload!((a: i64) % (b: &Series) -> Decimal { a % b[0] });

overload!((a: f64) + (b: &Series) -> Decimal { a + b[0] });

overload!((a: f64) - (b: &Series) -> Decimal { a - b[0] });

overload!((a: f64) * (b: &Series) -> Decimal { a * b[0] });

overload!((a: f64) / (b: &Series) -> Decimal { a / b[0] });

overload!((a: f64) % (b: &Series) -> Decimal { a % b[0] });

overload!((a: &Series) + (b: &Series) -> Decimal { a[0] + b[0] });

overload!((a: &Series) - (b: &Series) -> Decimal { a[0] - b[0] });

overload!((a: &Series) * (b: &Series) -> Decimal { a[0] * b[0] });

overload!((a: &Series) / (b: &Series) -> Decimal { a[0] / b[0] });

overload!((a: &Series) % (b: &Series) -> Decimal { a[0] % b[0] });

overload!(- (a: &Series) -> Decimal { -a[0]  });

/// A data series with reverse indexing.
///
/// # Examples
///
/// ```
/// use trading_maid::series::TimeSeries;
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
    fn eq(&self, other: &i32) -> bool {
        self.index(0) == &(*other as u64)
    }
}

impl PartialEq<u32> for &TimeSeries {
    fn eq(&self, other: &u32) -> bool {
        self.index(0) == &(*other as u64)
    }
}

impl PartialEq<i64> for &TimeSeries {
    fn eq(&self, other: &i64) -> bool {
        self.index(0) == &(*other as u64)
    }
}

impl PartialEq<u64> for &TimeSeries {
    fn eq(&self, other: &u64) -> bool {
        self.index(0) == other
    }
}

impl PartialEq<[u64]> for TimeSeries {
    fn eq(&self, other: &[u64]) -> bool {
        &self.inner == other
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

impl PartialOrd<[u64]> for TimeSeries {
    fn partial_cmp(&self, other: &[u64]) -> Option<Ordering> {
        self.inner.partial_cmp(other)
    }
}

impl PartialOrd for &TimeSeries {
    fn partial_cmp(&self, other: &&TimeSeries) -> Option<Ordering> {
        self.index(0).partial_cmp(other.index(0))
    }
}

overload!((a: &TimeSeries) + (b: i64) -> u64 { a[0] + b as u64 });

overload!((a: &TimeSeries) - (b: i64) -> u64 { a[0] - b as u64 });

overload!((a: &TimeSeries) * (b: i64) -> u64 { a[0] * b as u64 });

overload!((a: &TimeSeries) / (b: i64) -> u64 { a[0] / b as u64 });

overload!((a: &TimeSeries) % (b: i64) -> u64 { a[0] % b as u64 });

overload!((a: i64) + (b: &TimeSeries) -> u64 { a as u64 + b[0] });

overload!((a: i64) - (b: &TimeSeries) -> u64 { a as u64 - b[0] });

overload!((a: i64) * (b: &TimeSeries) -> u64 { a as u64 * b[0] });

overload!((a: i64) / (b: &TimeSeries) -> u64 { a as u64 / b[0] });

overload!((a: i64) % (b: &TimeSeries) -> u64 { a as u64 % b[0] });

overload!((a: &TimeSeries) + (b: &TimeSeries) -> u64 { a[0] + b[0] });

overload!((a: &TimeSeries) - (b: &TimeSeries) -> u64 { a[0] - b[0] });

overload!((a: &TimeSeries) * (b: &TimeSeries) -> u64 { a[0] * b[0] });

overload!((a: &TimeSeries) / (b: &TimeSeries) -> u64 { a[0] / b[0] });

overload!((a: &TimeSeries) % (b: &TimeSeries) -> u64 { a[0] % b[0] });
