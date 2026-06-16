import { LOCALE_DATE_FORMATS, TickMarkType } from './constants';

export function tickMarkFormatter(time, tickMarkType, locale) {
  let date;

  if (typeof time === 'string') {
    date = new Date(time);
  } else if (typeof time === 'number') {
    date = new Date(time);
  } else {
    date = new Date(time.year, time.month - 1, time.day);
  }

  switch (tickMarkType) {
    case TickMarkType.Year:
      return date.toLocaleString(locale, { year: 'numeric' }).slice(0, 4);

    case TickMarkType.Month:
      return date.toLocaleString(locale, { month: 'short' }).slice(0, 3);

    case TickMarkType.DayOfMonth:
      return date.toLocaleString(locale, { day: '2-digit' }).padStart(2, '0');

    case TickMarkType.Time:
      return date.toLocaleString(locale, { hour: '2-digit', minute: '2-digit' }).slice(0, 5);

    case TickMarkType.TimeWithSeconds:
      return date.toLocaleString(locale, { hour: '2-digit', minute: '2-digit', second: '2-digit' }).slice(0, 8);

    default:
      return null;
  }
}

export function timeFormatter(time, locale) {
  let date;

  if (typeof time === 'string') {
    date = new Date(time);
  } else if (typeof time === 'number') {
    date = new Date(time);
  } else {
    date = new Date(time.year, time.month - 1, time.day);
  }

  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  const hour = String(date.getHours()).padStart(2, '0');
  const minute = String(date.getMinutes()).padStart(2, '0');
  const second = String(date.getSeconds()).padStart(2, '0');
  const ampm = hour >= 12 ? 'PM' : 'AM';
  const hour12 = hour % 12 || 12;

  const dateFormat = LOCALE_DATE_FORMATS[locale] || 'yyyy-MM-dd HH:mm';

  return dateFormat
    .replace('yyyy', year)
    .replace('MM', month)
    .replace('dd', day)
    .replace('HH', hour)
    .replace('hh', hour12)
    .replace('mm', minute)
    .replace('ss', second)
    .replace('A', ampm);
}

export function getTimeRange(time, level) {
  if (typeof time !== 'number' || isNaN(time)) {
    throw new Error('Invalid timestamp');
  }
  const dt = new Date(time);
  if (isNaN(dt.getTime())) {
    throw new Error('Invalid date');
  }

  switch (level) {
    case '1m': {
      const start = new Date(dt);
      start.setUTCSeconds(0);
      start.setUTCMilliseconds(0);
      const next = new Date(start.getTime() + 60000);
      return [start.getTime(), next.getTime()];
    }
    case '3m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 3) * 60000);
      const next = new Date(start.getTime() + 180000);
      return [start.getTime(), next.getTime()];
    }
    case '5m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 5) * 60000);
      const next = new Date(start.getTime() + 300000);
      return [start.getTime(), next.getTime()];
    }
    case '15m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 15) * 60000);
      const next = new Date(start.getTime() + 900000);
      return [start.getTime(), next.getTime()];
    }
    case '30m': {
      const temp = new Date(dt);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const mins = temp.getUTCMinutes();
      const start = new Date(temp.getTime() - (mins % 30) * 60000);
      const next = new Date(start.getTime() + 1800000);
      return [start.getTime(), next.getTime()];
    }
    case '1h': {
      const start = new Date(dt);
      start.setUTCMinutes(0);
      start.setUTCSeconds(0);
      start.setUTCMilliseconds(0);
      const next = new Date(start.getTime() + 3600000);
      return [start.getTime(), next.getTime()];
    }
    case '2h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 2) * 3600000);
      const next = new Date(start.getTime() + 7200000);
      return [start.getTime(), next.getTime()];
    }
    case '4h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 4) * 3600000);
      const next = new Date(start.getTime() + 14400000);
      return [start.getTime(), next.getTime()];
    }
    case '6h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 6) * 3600000);
      const next = new Date(start.getTime() + 21600000);
      return [start.getTime(), next.getTime()];
    }
    case '12h': {
      const temp = new Date(dt);
      temp.setUTCMinutes(0);
      temp.setUTCSeconds(0);
      temp.setUTCMilliseconds(0);
      const hrs = temp.getUTCHours();
      const start = new Date(temp.getTime() - (hrs % 12) * 3600000);
      const next = new Date(start.getTime() + 43200000);
      return [start.getTime(), next.getTime()];
    }
    case '1d': {
      const start = new Date(dt);
      start.setUTCHours(0, 0, 0, 0);
      const next = new Date(start.getTime() + 86400000);
      return [start.getTime(), next.getTime()];
    }
    case '3d': {
      const dayStart = new Date(dt);
      dayStart.setUTCHours(0, 0, 0, 0);
      const ceStart = new Date(Date.UTC(1, 0, 1));
      const msDiff = dayStart.getTime() - ceStart.getTime();
      const days = Math.floor(msDiff / 86400000) + 1;
      const startOrdinal = Math.floor(days / 3) * 3;
      const startMs = ceStart.getTime() + (startOrdinal - 1) * 86400000;
      const start = new Date(startMs);
      const next = new Date(startMs + 259200000);
      return [start.getTime(), next.getTime()];
    }
    case '1w': {
      const start = new Date(dt);
      start.setUTCHours(0, 0, 0, 0);
      const weekday = start.getUTCDay();
      const daysToSubtract = (weekday + 6) % 7;
      start.setUTCDate(start.getUTCDate() - daysToSubtract);
      const next = new Date(start.getTime() + 604800000);
      return [start.getTime(), next.getTime()];
    }
    case '1mo': {
      const start = new Date(dt);
      start.setUTCHours(0, 0, 0, 0);
      start.setUTCDate(1);
      const next = new Date(start.getTime());
      next.setUTCMonth(next.getUTCMonth() + 1);
      next.setUTCHours(0, 0, 0, 0);
      return [start.getTime(), next.getTime()];
    }
    default:
      throw new Error('Unknown level: ' + level);
  }
}
