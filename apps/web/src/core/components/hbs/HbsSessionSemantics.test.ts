import { afterEach, expect, it } from "vitest";
import { createTextDocumentAdapter } from "@research-assistant/core/hbs";
import { HbsSession } from "@research-assistant/core/hbs";
import { DocumentVersionConflict, type DocumentCheckpoint, type DocumentCommit, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { directTextDiff } from "./codeMirrorHbsPolicy";
const initial = "title\n\nsample 120\nscore 80%\n\nmethod old\n\nnext old";
const disposals: (() => void)[]=[];
afterEach(()=>disposals.splice(0).forEach(dispose=>dispose()));
const settle=async()=>{for(let n=0;n<60;n++)await Promise.resolve();};
async function fixture(){
  const adapter=createTextDocumentAdapter(directTextDiff);
  let server:DocumentSnapshot<string>={version:1,saved:initial,baseline:initial},checkpoint:DocumentCheckpoint<string>|undefined;
  let hold=false;const releases:(()=>void)[]=[];const requests:DocumentCommit<string>[]=[];
  const options={adapter,requestId:(n:number)=>`r${n}`,preservation:{load:()=>checkpoint,save:(value:DocumentCheckpoint<string>)=>{checkpoint=structuredClone(value);}},port:{read:async()=>server,commit:async(request:DocumentCommit<string>)=>{
    requests.push(request);if(hold)await new Promise<void>(resolve=>releases.push(resolve));
    if(request.expectedVersion!==server.version)throw new DocumentVersionConflict(server,request.id);
    server={version:server.version+1,saved:request.saved,baseline:request.baseline};return server;
  }}};
  const session=await HbsSession.open(options);disposals.push(()=>session.dispose());
  return {session,adapter,options,requests,get server(){return server;},get checkpoint(){return checkpoint;},
    external:(before:string,after:string)=>{server={...server,version:server.version+1,saved:server.saved.replace(before,after)};},
    hold:()=>{hold=true;},release:()=>{hold=false;releases.splice(0).forEach(resolve=>resolve());},
    state:()=>session.getSnapshot()};
}
async function conflict(){const f=await fixture();f.external("80%","85%");await f.session.syncLatest();f.session.edit(f.state().content.replace("85%","91%"),f.state().baselineProjection);f.external("85%","90%");f.external("next old","next new");await f.session.save();return f;}
it("retains separate pending ranges and exposes only the real incoming range",async()=>{
  const f=await conflict();expect(f.state().content).toContain("next new");expect(f.state().changes).toHaveLength(2);
  const c=f.state().conflicts[0]!;expect(c.text).toBeDefined();
  expect(String(c.after).slice(c.text!.fromB,c.text!.toB)).toBe("score 91%");
  f.external("next new","next newest");await f.session.syncLatest();expect(f.state().conflicts).toHaveLength(1);expect(f.state().content).toContain("next newest");
});
it("keeps new pending through a held Keep receipt and later candidate input",async()=>{
  const f=await fixture();f.external("80%","85%");await f.session.syncLatest();f.hold();
  f.session.perform(()=>f.session.resolve(f.state().changes[0]!.id,"keep"));await settle();
  expect(f.state().baselineProjection).toContain("80%");expect(f.state().changes).toHaveLength(1);
  f.session.edit(f.state().content.replace("85%","92%"));f.release();await settle();
  expect(f.state().baselineProjection).toContain("85%");expect(f.state().content).toContain("92%");
  await f.session.save();expect(f.server.saved).toContain("92%");expect(f.server.baseline).toContain("85%");
});
it("requeues a cancelled local review on redo",async()=>{
  const f=await conflict();const next=f.state().changes.find(change=>change.after!==change.before && String(change.after).includes("next new"));
  const entry=f.session.perform(()=>f.session.review(f.state().content,f.state().baselineProjection.replace("next old","next new")));
  await settle();f.session.restore(entry,"before");expect(f.state().queued).toBe(1); // only the save requested before conflict
  f.session.restore(entry,"after");expect(f.state().queued).toBe(2);expect(next).toBeDefined();
});
it("confirms only the viewed old new when a newer remote candidate rejects Keep",async()=>{
  const f=await fixture();f.external("80%","85%");await f.session.syncLatest();f.hold();
  f.session.perform(()=>f.session.resolve(f.state().changes[0]!.id,"keep"));await settle();
  f.external("85%","90%");f.release();await settle();
  expect(f.state().content).toContain("90%");expect(f.state().conflicts).toHaveLength(0);
  expect(f.server.baseline).toContain("85%");expect(f.server.saved).toContain("90%");expect(f.state().changes).toHaveLength(1);
});

it("keeps conflict choices local to their range and keeps ordinary input",async()=>{
  const f=await conflict();f.session.edit(f.state().content.replace("method old","method typed"),f.state().baselineProjection.replace("method old","method typed"));
  f.session.resolve(f.state().conflicts[0]!.id,"both");await settle();
  expect(f.state().content.match(/title/gu)).toHaveLength(1);expect(f.state().content).toContain("method typed");expect(f.state().content).toContain("next new");
  expect(f.state().baselineProjection).toContain("80%");expect(f.state().baselineProjection).toContain("next old");
});

it("shows a queued Undo proposal against later typing and retains it while editing",async()=>{
  const f=await fixture();f.external("80%","85%");await f.session.syncLatest();
  f.session.edit(f.state().content.replace("85%","91%"),f.state().baselineProjection);
  f.hold();void f.session.save();await settle();
  f.session.perform(()=>f.session.review(f.state().content.replace("91%","80%"),f.state().baselineProjection));
  f.session.edit(f.state().content.replace("80%","92%"),f.state().baselineProjection);
  f.release();await settle();
  expect(f.state().content).toContain("92%");
  let change=f.state().conflicts[0]!;
  expect(f.adapter.textIncoming!(change,f.state().content,f.server.baseline,f.server.saved).text).toBe("80%");
  f.session.edit(f.state().content.replace("92%","93%"),f.state().baselineProjection);
  change=f.state().conflicts[0]!;
  expect(f.adapter.textIncoming!(change,f.state().content,f.server.baseline,f.server.saved).text).toBe("80%");
  f.external("91%","95%");await f.session.syncLatest();
  expect(f.state().conflicts).toHaveLength(1);
  expect(f.adapter.textIncoming!(f.state().conflicts[0]!,f.state().content,f.server.baseline,f.server.saved).text).toBe("score 95%");
  f.session.resolve(f.state().conflicts[0]!.id,"incoming");
  expect(f.state().content).toContain("95%");
  expect(f.adapter.textIncoming!(f.state().conflicts[0]!,f.state().content,f.server.baseline,f.server.saved).text).toBe("80%");
  change=f.state().conflicts[0]!;
  f.session.resolve(change.id,"incoming");await f.session.save();
  expect(f.server.saved).toContain("80%");expect(f.server.baseline).toContain("80%");
});

it("does not authorize a newer incoming candidate through an old conflict identity",async()=>{
  const f=await conflict(),old=f.state().conflicts[0]!.id;
  f.external("90%","95%");await f.session.syncLatest();
  expect(f.state().conflicts[0]!.id).not.toBe(old);
  f.session.resolve(old,"incoming");
  expect(f.state().content).toContain("91%");expect(f.state().message).toContain("MODEL_RANGE_CHANGED");
  expect(f.state().conflicts).toHaveLength(1);
});

it("replays a committed conflict Keep without being blocked by its restored incoming",async()=>{
  const f=await conflict();
  const entry=f.session.perform(()=>f.session.resolve(f.state().conflicts[0]!.id,"keep"));await settle();
  expect(f.server.baseline).toContain("91%");
  f.session.restore(entry,"before");await settle();
  expect(f.server.baseline).toContain("80%");expect(f.state().conflicts).toHaveLength(1);
  expect(f.state().queued).toBe(0);expect(f.state().message).not.toContain("MODEL_UNRESOLVED");
  f.session.restore(entry,"after");await settle();
  expect(f.server.baseline).toContain("91%");expect(f.state().conflicts).toHaveLength(0);
  expect(f.server.baseline).toContain("next old");expect(f.state().content).toContain("next new");
});
