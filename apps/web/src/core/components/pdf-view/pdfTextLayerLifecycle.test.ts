// @vitest-environment happy-dom
import {afterEach,expect,it,vi} from "vitest";

// Exercise the shipped builder and its document listener, not an application
// mock. WebKit can deliver a selection callback queued before the last layer
// was canceled while focus moves to a retained note/source editor.
it.each(["legacy","modern"])("ignores a late selection event after the final real PDF text layer is canceled (%s)",async variant=>{
  // Empty text needs no raster drawing; provide only the platform canvas port.
  vi.spyOn(HTMLCanvasElement.prototype,"getContext").mockImplementation(()=>({} as CanvasRenderingContext2D));
  if(variant==="legacy")await import("pdfjs-dist/legacy/build/pdf.mjs");
  else await import("pdfjs-dist");
  const {TextLayerBuilder}=(variant==="legacy"?await import("pdfjs-dist/legacy/web/pdf_viewer.mjs"):await import("pdfjs-dist/web/pdf_viewer.mjs")) as unknown as {
    TextLayerBuilder: typeof import("pdfjs-dist/types/web/text_layer_builder").TextLayerBuilder;
  };
  let late:EventListener|undefined;
  const add=document.addEventListener.bind(document);
  vi.spyOn(document,"addEventListener").mockImplementation((type,listener,options)=>{
    if(type==="selectionchange"&&typeof listener==="function")late=listener;
    add(type,listener,options);
  });
  const layer=new TextLayerBuilder({pdfPage:{streamTextContent:()=>({items:[],styles:{}})} as unknown as ConstructorParameters<typeof TextLayerBuilder>[0]["pdfPage"]});
  document.body.append(layer.div);
  try {
    await layer.render({viewport:{scale:1,rotation:0,width:100,height:100,rawDims:{pageWidth:100,pageHeight:100,pageX:0,pageY:0}}} as unknown as Parameters<typeof layer.render>[0]);
    expect(late).toBeTypeOf("function");
    layer.cancel();
    const editor=document.createElement("div");editor.textContent="Synthetic unrelated editor selection";document.body.append(editor);
    const selection=document.getSelection()!;
    const range=document.createRange();range.selectNodeContents(editor);selection.removeAllRanges();selection.addRange(range);
    expect(selection.rangeCount).toBe(1);
    expect(()=>late!.call(document,new Event("selectionchange"))).not.toThrow();
    selection.removeAllRanges();editor.remove();
  } finally {layer.cancel();layer.div.remove();}
});
afterEach(()=>vi.restoreAllMocks());
