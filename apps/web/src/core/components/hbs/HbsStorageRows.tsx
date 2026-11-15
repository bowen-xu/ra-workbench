import "./HbsStorageRows.css";
export interface HbsStoredTable {
  readonly table:string;
  readonly columns:readonly string[];
  readonly rows:readonly Readonly<Record<string,string|number|null>>[];
}
/** Render persisted store entries or repository rows without re-encoding values. */
export function HbsStorageRows({tables}:{readonly tables:readonly HbsStoredTable[]}):React.JSX.Element{
  return <div className="hbs-storage-rows">{tables.map(table=><section key={table.table} aria-label={`存储 ${table.table}`}>
    <header>{table.table} · {table.rows.length} 行</header>
    {table.rows.length ? <table><thead><tr>{table.columns.map(column=><th key={column}>{column}</th>)}</tr></thead>
      <tbody>{table.rows.map((row,index)=><tr key={index}>{table.columns.map(column=><td key={column}><pre>{row[column]===null ? "NULL" : String(row[column]??"")}</pre></td>)}</tr>)}</tbody>
    </table> : <p>暂无记录</p>}
  </section>)}</div>;
}
