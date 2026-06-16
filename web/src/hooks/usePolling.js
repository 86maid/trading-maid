import { useEffect } from 'react';

export function usePolling(refreshData) {
  useEffect(() => {
    const hash = window.hash;
    const state = window.state;

    if (!hash || state === undefined) return;

    const interval = setInterval(() => {
      fetch(`/update/${hash}/${state}`)
        .then((response) => {
          if (response.status === 304) {
            return null;
          }
          if (response.status === 205) {
            window.location.reload();
            return null;
          }
          return response.text();
        })
        .then((text) => {
          if (!text) return;
          new Function(text)();
          refreshData();
        })
        .catch(() => {
          /* swallow fetch errors */
        });
    }, 100);

    return () => clearInterval(interval);
  }, [refreshData]);
}
