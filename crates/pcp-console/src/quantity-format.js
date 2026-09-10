// Display only: callers keep raw integers for accounting, forms and sorting.
export function compactQuantity(value, locale = "en-US") {
  if (value == null || !Number.isFinite(Number(value))) return "—";
  const number = Number(value);
  const units = /^zh(?:-|$)/i.test(locale)
    ? [[1, ""], [1e4, "万"], [1e8, "亿"]]
    : [[1, ""], [1e3, "k"], [1e6, "m"], [1e9, "b"]];
  let index = 0;
  while (index + 1 < units.length && Math.abs(number) >= units[index + 1][0]) index++;
  // Promote rounded boundary values instead of displaying 1000k or 10000万.
  while (index + 1 < units.length && Number((Math.abs(number) / units[index][0]).toFixed(2)) >= units[index + 1][0] / units[index][0]) index++;
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 2, useGrouping: index === 0 }).format(number / units[index][0]) + units[index][1];
}

export function exactQuantity(value, locale = "en-US") {
  return value == null || !Number.isFinite(Number(value)) ? "—" : new Intl.NumberFormat(locale, {maximumFractionDigits: 20}).format(Number(value));
}
