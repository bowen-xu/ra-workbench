import { SelectPopover } from "./overlay/OverlayPopup";
import { Select, SelectItem, SelectProvider } from "@ariakit/react";
import { Check, ChevronDown } from "lucide-react";
import type { CitationStyle } from "@research-assistant/core/citation";

const options = [{value:"apa",label:"APA"},{value:"nature",label:"Nature"},{value:"ieee",label:"IEEE"}] as const;

/** Settings-template presentation with Ariakit owning keyboard, placement and focus. */
export function CitationStyleSelect({value,onChange}:{
  readonly value: CitationStyle;
  readonly onChange: (value: CitationStyle) => void;
}): React.JSX.Element {
  return <SelectProvider value={value} placement="bottom-end" setValue={next=>{if(next==="apa"||next==="nature"||next==="ieee")onChange(next);}}>
    <Select className="citation-style-select" aria-label="引用样式">
      <span>{options.find(option=>option.value===value)?.label}</span><ChevronDown size={15} aria-hidden="true" />
    </Select>
    <SelectPopover className="citation-style-options" aria-label="引用样式" portal fixed gutter={4}>
      {options.map(option=><SelectItem className="citation-style-option" key={option.value} value={option.value}>
        <span>{option.label}</span>{option.value===value&&<Check size={19} aria-hidden="true" />}
      </SelectItem>)}
    </SelectPopover>
  </SelectProvider>;
}
