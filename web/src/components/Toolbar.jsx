import React from 'react';
import { Select, Switch, Tooltip, Flex, Divider } from 'antd';
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
    <Flex
      gap="small"
      align="center"
      style={{
        padding: '8px 10px',
        borderBottom: '1px solid var(--border-color)',
        backgroundColor: 'var(--card-background-color)',
      }}
    >
      <Select
        value={currentSymbol}
        onChange={setCurrentSymbol}
        options={symbolOptions}
        style={{ minWidth: 110 }}
        size="middle"
        popupMatchSelectWidth={false}
      />
      <Divider type="vertical" />
      <Select
        value={currentLevel}
        onChange={setCurrentLevel}
        options={levelOptions}
        style={{ minWidth: 80 }}
        size="middle"
        popupMatchSelectWidth={false}
      />
      <Divider type="vertical" />
      <Select
        value={theme}
        onChange={setTheme}
        options={themeOptions}
        style={{ minWidth: 90 }}
        size="middle"
        popupMatchSelectWidth={false}
      />
      <Divider type="vertical" />
      <Tooltip title={isZh ? '价格磁铁' : 'Price Magnet'}>
        <Switch
          checked={magnet}
          onChange={setMagnet}
          checkedChildren="🧲"
          unCheckedChildren="🧲"
        />
      </Tooltip>
      <Divider type="vertical" />
      <Tooltip title={isZh ? '成交量' : 'Volume'}>
        <Switch
          checked={showVolume}
          onChange={setShowVolume}
          checkedChildren="📊"
          unCheckedChildren="📊"
        />
      </Tooltip>
    </Flex>
  );
}
