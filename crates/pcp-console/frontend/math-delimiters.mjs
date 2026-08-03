const delimiters = [
  {
    pattern: /(?<!\\)\\\[([\s\S]+?)\\\]/g,
    display: true,
  },
  {
    pattern: /(?<!\\)\$\$([\s\S]+?)(?<!\\)\$\$/g,
    display: true,
  },
  {
    pattern: /(?<!\\)\\\(([\s\S]+?)\\\)/g,
    display: false,
  },
  {
    pattern: /(?<!\\)\$([^$\n]+?)(?<!\\)\$/g,
    display: false,
  },
];

export function protectMath(markdown) {
  const math = [];
  let protectedMarkdown = String(markdown || "");

  for (const delimiter of delimiters) {
    protectedMarkdown = protectedMarkdown.replace(
      delimiter.pattern,
      (_, expression) => mathToken(math, expression, delimiter.display),
    );
  }

  return { protectedMarkdown, math };
}

function mathToken(math, expression, display) {
  const index = math.push({ expression: expression.trim(), display }) - 1;
  return `<span data-pcp-math="${index}"></span>`;
}
