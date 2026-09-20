// A source date is not an instant. Keep its precision rather than interpreting
// YYYY-MM-DD as UTC midnight and fabricating a local clock time.
export function formatTimestamp(value, locale, options = {}) {
  if (value == null || value === "") return "-";
  const source = String(value);
  if (/^\d{4}-\d{2}-\d{2}$/.test(source)) return source;
  const date = new Date(typeof value === "number" ? value : source);
  return Number.isNaN(date.getTime()) ? source : date.toLocaleString(locale, options);
}
