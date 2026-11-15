/** A structural inspector renders values as containers. Internal highlighting
 * may refine a string leaf; it never splits serialized markup or JSON syntax. */
export function StructuredValueView({ value }: {readonly value: unknown}): React.JSX.Element {
  if (value === undefined) return <span>（移除）</span>;
  if (value === null || typeof value !== "object") return <span>{String(value)}</span>;
  if (Array.isArray(value)) return <ol>{value.map((child,index)=><li key={index}><StructuredValueView value={child}/></li>)}</ol>;
  return <dl>{Object.entries(value).map(([key,child])=><div key={key}><dt>{key}</dt><dd><StructuredValueView value={child}/></dd></div>)}</dl>;
}
