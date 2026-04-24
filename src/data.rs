use anyhow::{Context, bail};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use std::{
    fmt::Display,
    fs::{self, File},
    io::{BufReader, BufWriter},
    ops::Deref,
    path::Path,
    str::FromStr,
    sync::Arc,
};

use crate::util::resample;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Minute1,
    Minute3,
    Minute5,
    Minute15,
    Minute30,
    Hour1,
    Hour2,
    Hour4,
    Hour6,
    Hour12,
    Day1,
    Day3,
    Week1,
    Month1,
}

impl Level {
    pub fn from_interval_minutes(minutes: u64) -> Option<Level> {
        match minutes {
            1 => Some(Level::Minute1),
            3 => Some(Level::Minute3),
            5 => Some(Level::Minute5),
            15 => Some(Level::Minute15),
            30 => Some(Level::Minute30),
            60 => Some(Level::Hour1),
            120 => Some(Level::Hour2),
            240 => Some(Level::Hour4),
            360 => Some(Level::Hour6),
            720 => Some(Level::Hour12),
            1440 => Some(Level::Day1),
            4320 => Some(Level::Day3),
            10080 => Some(Level::Week1),
            _ if minutes > 10080 => Some(Level::Month1),
            _ => None,
        }
    }

    pub fn interval_minutes(&self) -> u64 {
        match self {
            Level::Minute1 => 1,
            Level::Minute3 => 3,
            Level::Minute5 => 5,
            Level::Minute15 => 15,
            Level::Minute30 => 30,
            Level::Hour1 => 60,
            Level::Hour2 => 120,
            Level::Hour4 => 240,
            Level::Hour6 => 360,
            Level::Hour12 => 720,
            Level::Day1 => 1440,
            Level::Day3 => 4320,
            Level::Week1 => 10080,
            Level::Month1 => 0,
        }
    }

    pub fn interval_millis(&self) -> u64 {
        self.interval_minutes() * 60000
    }

    pub fn is_valid_sampling_target(&self, target_level: Level) -> bool {
        target_level
            .interval_minutes()
            .is_multiple_of(self.interval_minutes())
    }
}

impl Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Level::Minute1 => "1m",
            Level::Minute3 => "3m",
            Level::Minute5 => "5m",
            Level::Minute15 => "15m",
            Level::Minute30 => "30m",
            Level::Hour1 => "1h",
            Level::Hour2 => "2h",
            Level::Hour4 => "4h",
            Level::Hour6 => "6h",
            Level::Hour12 => "12h",
            Level::Day1 => "1d",
            Level::Day3 => "3d",
            Level::Week1 => "1w",
            Level::Month1 => "1mo",
        })
    }
}

impl FromStr for Level {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1m" => Ok(Level::Minute1),
            "3m" => Ok(Level::Minute3),
            "5m" => Ok(Level::Minute5),
            "15m" => Ok(Level::Minute15),
            "30m" => Ok(Level::Minute30),
            "1h" => Ok(Level::Hour1),
            "2h" => Ok(Level::Hour2),
            "4h" => Ok(Level::Hour4),
            "6h" => Ok(Level::Hour6),
            "12h" => Ok(Level::Hour12),
            "1d" => Ok(Level::Day1),
            "3d" => Ok(Level::Day3),
            "1w" => Ok(Level::Week1),
            "1mo" => Ok(Level::Month1),
            _ => bail!("invalid level: {}", s),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct KLine {
    pub time: u64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub symbol: String,
    #[serde_as(as = "DisplayFromStr")]
    pub level: Level,
    pub min_size: Decimal,
    pub min_notional: Decimal,
    pub tick_size: Decimal,
    pub maker_fee: Decimal,
    pub taker_fee: Decimal,
    pub maintenance: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceInner {
    pub metadata: Metadata,
    pub data: Vec<KLine>,
}

#[derive(Debug, Clone)]
pub struct DataSource(Arc<DataSourceInner>);

impl Deref for DataSource {
    type Target = DataSourceInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DataSource {
    pub fn new(metadata: Metadata, data: Vec<KLine>) -> Self {
        DataSource(Arc::new(DataSourceInner { metadata, data }))
    }

    pub fn resample(&self, level: Level) -> anyhow::Result<Self> {
        if !self.metadata.level.is_valid_sampling_target(level) {
            bail!(
                "invalid sampling target level: min_level: {}, target_level: {}",
                self.metadata.level,
                level
            );
        }

        let mut metadata = self.metadata.clone();
        let data = resample(&self.data, level)?;

        metadata.level = level;

        Ok(Self::new(metadata, data))
    }

    pub fn range(&self, start_time: u64, end_time: u64) -> Self {
        Self::new(
            self.metadata.clone(),
            self.data
                .iter()
                .filter(|v| v.time >= start_time && v.time <= end_time)
                .cloned()
                .collect(),
        )
    }

    pub fn is_sorted_by_time(&self) -> bool {
        self.data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
    }

    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .context("no extension")?;

        match extension {
            "json" => Self::from_json_file(path),
            "csv" => Self::from_csv_file(path),
            "bin" => Self::from_bin_file(path),
            v => bail!("invalid extension: {}", v),
        }
    }

    pub fn from_json_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut inner =
            serde_json::from_reader::<_, DataSourceInner>(BufReader::new(File::open(path)?))?;

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_json_text(text: impl AsRef<str>) -> anyhow::Result<Self> {
        let mut inner = serde_json::from_str::<DataSourceInner>(text.as_ref())?;

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_csv_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_path(path)?;

        let metadata = reader.deserialize().next().context("no metadata")??;

        let data = reader.deserialize().collect::<Result<_, _>>()?;

        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_csv_text(text: impl AsRef<str>) -> anyhow::Result<Self> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_reader(text.as_ref().as_bytes());

        let metadata = reader.deserialize().next().context("no metadata")??;

        let data = reader.deserialize().collect::<Result<_, _>>()?;

        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_bin_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut inner = bincode::deserialize::<DataSourceInner>(&fs::read(path)?)?;

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_file_metadata(path: impl AsRef<Path>, metadata: Metadata) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .context("no extension")?;

        match extension {
            "json" => Self::from_json_metadata(path, metadata),
            "csv" => Self::from_csv_metadata(path, metadata),
            "bin" => Self::from_bin_metadata(path, metadata),
            v => bail!("invalid extension: {}", v),
        }
    }

    pub fn from_json_metadata(path: impl AsRef<Path>, metadata: Metadata) -> anyhow::Result<Self> {
        let data: Vec<KLine> = serde_json::from_reader(BufReader::new(File::open(path)?))?;
        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_json_metadata_text(
        text: impl AsRef<str>,
        metadata: Metadata,
    ) -> anyhow::Result<Self> {
        let data: Vec<KLine> = serde_json::from_str(text.as_ref())?;
        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_csv_metadata(path: impl AsRef<Path>, metadata: Metadata) -> anyhow::Result<Self> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_path(path)?;

        let data = reader.deserialize().collect::<Result<Vec<KLine>, _>>()?;
        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_csv_metadata_text(
        text: impl AsRef<str>,
        metadata: Metadata,
    ) -> anyhow::Result<Self> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_reader(text.as_ref().as_bytes());

        let data = reader.deserialize().collect::<Result<Vec<KLine>, _>>()?;
        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn from_bin_metadata(path: impl AsRef<Path>, metadata: Metadata) -> anyhow::Result<Self> {
        let data: Vec<KLine> = bincode::deserialize(&fs::read(path)?)?;
        let mut inner = DataSourceInner { metadata, data };

        if !inner
            .data
            .windows(2)
            .all(|pair| pair[0].time <= pair[1].time)
        {
            inner.data.sort_by_key(|v| v.time);
        }

        Ok(DataSource(Arc::new(inner)))
    }

    pub fn write_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .context("no extension")?;

        match extension {
            "json" => self.write_json_file(path),
            "csv" => self.write_csv_file(path),
            "bin" => self.write_bin_file(path),
            v => bail!("invalid extension: {}", v),
        }
    }

    pub fn write_json_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        serde_json::to_writer(BufWriter::new(File::create(path)?), self)?;

        Ok(())
    }

    pub fn write_csv_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let mut writer = csv::WriterBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_path(path)?;

        writer.serialize(&self.metadata)?;

        for v in &self.data {
            writer.serialize(v)?;
        }

        Ok(())
    }

    pub fn write_bin_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        fs::write(path, bincode::serialize(&*self.0)?)?;

        Ok(())
    }
}

impl Serialize for DataSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DataSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(DataSource(Arc::new(DataSourceInner::deserialize(
            deserializer,
        )?)))
    }
}

#[derive(Debug, Clone)]
pub struct KLineBuffer<const N: usize> {
    pub time: Vec<u64>,
    pub open: Vec<Decimal>,
    pub high: Vec<Decimal>,
    pub low: Vec<Decimal>,
    pub close: Vec<Decimal>,
    pub volume: Vec<Decimal>,
}

impl<const N: usize> Default for KLineBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> KLineBuffer<N> {
    pub fn new() -> Self {
        Self {
            time: Vec::new(),
            open: Vec::new(),
            high: Vec::new(),
            low: Vec::new(),
            close: Vec::new(),
            volume: Vec::new(),
        }
    }

    pub fn push(&mut self, k: KLine) {
        if N > 0 && self.time.len() >= N {
            let remove = self.time.len() + 1 - N;

            self.time.drain(0..remove);
            self.open.drain(0..remove);
            self.high.drain(0..remove);
            self.low.drain(0..remove);
            self.close.drain(0..remove);
            self.volume.drain(0..remove);
        }

        self.time.push(k.time);
        self.open.push(k.open);
        self.high.push(k.high);
        self.low.push(k.low);
        self.close.push(k.close);
        self.volume.push(k.volume);
    }

    pub fn extend(&mut self, k: Vec<KLine>) {
        if k.is_empty() {
            return;
        }

        if N == 0 {
            let i = k.iter();

            self.time.extend(i.clone().map(|v| v.time));
            self.open.extend(i.clone().map(|v| v.open));
            self.high.extend(i.clone().map(|v| v.high));
            self.low.extend(i.clone().map(|v| v.low));
            self.close.extend(i.clone().map(|v| v.close));
            self.volume.extend(i.clone().map(|v| v.volume));
            return;
        }

        let incoming = if k.len() > N {
            &k[k.len() - N..]
        } else {
            &k[..]
        };

        let need_remove = self
            .time
            .len()
            .saturating_add(incoming.len())
            .saturating_sub(N);

        if need_remove > 0 {
            self.time.drain(0..need_remove);
            self.open.drain(0..need_remove);
            self.high.drain(0..need_remove);
            self.low.drain(0..need_remove);
            self.close.drain(0..need_remove);
            self.volume.drain(0..need_remove);
        }

        let i = incoming.iter();

        self.time.extend(i.clone().map(|v| v.time));
        self.open.extend(i.clone().map(|v| v.open));
        self.high.extend(i.clone().map(|v| v.high));
        self.low.extend(i.clone().map(|v| v.low));
        self.close.extend(i.clone().map(|v| v.close));
        self.volume.extend(i.clone().map(|v| v.volume));
    }
}
