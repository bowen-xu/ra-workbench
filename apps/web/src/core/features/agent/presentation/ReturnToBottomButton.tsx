import {ChevronDown} from "lucide-react";
import {ControlHint} from "../../../public/overlay";
import {WaveDots} from "../../../public/ui";
import "./ReturnToBottomButton.css";

export function ReturnToBottomButton({visible, running, onClick}: {
  readonly visible: boolean;
  readonly running: boolean;
  readonly onClick: () => void;
}) {
  // Keep the zero-height anchor mounted for both opacity directions. Hidden
  // controls become inert immediately, before the visual fade has finished.
  return <div className="writing-agent-scroll-bottom-anchor" data-visible={visible} aria-hidden={!visible} inert={!visible}>
    <ControlHint content="回到底部"><button aria-label="回到底部" className="writing-agent-scroll-bottom"
      data-running={running} disabled={!visible} type="button" onClick={onClick}>
      {running && <WaveDots className="writing-agent-scroll-bottom-dots" durationSeconds={1} waveDurationSeconds={0.62} amplitudePx={0.8} dotSizePx={3} startDirection="down" />}
      <ChevronDown aria-hidden="true" className="writing-agent-scroll-bottom-arrow" size={16}/>
    </button></ControlHint>
  </div>;
}
