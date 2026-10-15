'use strict';
// Byte buffering preserves UTF-8 and JSON across log-tail boundaries.
class ProgressDecoder {
  constructor() { this.pending = Buffer.alloc(0); }
  push(bytes) {
    this.pending = Buffer.concat([this.pending, bytes]);
    const events = [];
    let end;
    while ((end = this.pending.indexOf(10)) >= 0) {
      const line = this.pending.subarray(0, end).toString('utf8');
      this.pending = this.pending.subarray(end + 1);
      const marker = '[TBuildToolProgress] ';
      const start = line.indexOf(marker);
      if (start < 0) continue;
      try {
        const v = JSON.parse(line.slice(start + marker.length));
        if (v.version !== 1 || typeof v.stage !== 'string' || !v.stage.trim()) continue;
        const pct = typeof v.pct === 'number' && Number.isFinite(v.pct) && v.pct >= 0
          ? Math.round(Math.min(100, v.pct)) : null;
        events.push({ stage: v.stage, pct });
      } catch { /* Ordinary/malformed logs are not progress. */ }
    }
    if (this.pending.length > 1024 * 1024) this.pending = Buffer.alloc(0);
    return events;
  }
}
module.exports = { ProgressDecoder };
