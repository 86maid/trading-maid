import React from 'react';
import { Select, Switch, Tooltip } from 'antd';
import { useTradingData } from '../context/TradingDataContext';

export default function Toolbar() {
  const {
    symbolList,
    levelList,
    currentSymbol,
    currentLevel,
    setCurrentSymbol,
    setCurrentLevel,
    theme,
    setTheme,
    magnet,
    setMagnet,
    showVolume,
    setShowVolume,
    isZh,
  } = useTradingData();

  const symbolOptions = symbolList.map((s) => ({ value: s, label: s }));
  const levelOptions = levelList.map((l) => ({ value: l, label: l }));
  const themeOptions = [
    { value: 'dark', label: isZh ? '暗色' : 'Dark' },
    { value: 'light', label: isZh ? '浅色' : 'Light' },
  ];

  return (
    <div className="toolbar">
      <Select
        value={currentSymbol}
        onChange={setCurrentSymbol}
        options={symbolOptions}
        style={{ minWidth: 110 }}
        size="middle"
        popupMatchSelectWidth={false}
      />
      <div className="divider" />
      <Select
        value={currentLevel}
        onChange={setCurrentLevel}
        options={levelOptions}
        style={{ minWidth: 80 }}
        size="middle"
        popupMatchSelectWidth={false}
      />
      <div className="divider" />
      <Select
        value={theme}
        onChange={setTheme}
        options={themeOptions}
        style={{ minWidth: 90 }}
        size="middle"
        popupMatchSelectWidth={false}
      />
      <div className="divider" />
      <Tooltip title={isZh ? '价格磁铁' : 'Price Magnet'}>
        <Switch
          checked={magnet}
          onChange={setMagnet}
          checkedChildren="🧲"
          unCheckedChildren="🧲"
        />
      </Tooltip>
      <div className="divider" />
      <Tooltip title={isZh ? '成交量' : 'Volume'}>
        <Switch
          checked={showVolume}
          onChange={setShowVolume}
          checkedChildren="📊"
          unCheckedChildren="📊"
        />
      </Tooltip>
    </div>
  );
}
