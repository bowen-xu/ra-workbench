import { expectTypeOf, it } from "vitest";
import type { DoclingGateway, DoclingParserGateway } from "./docling";
import type { getDoclingParserGateway } from "../public/docling";

it("exposes parse tasks and results without model or service administration", () => {
  expectTypeOf<DoclingGateway>().toExtend<DoclingParserGateway>();
  expectTypeOf<ReturnType<typeof getDoclingParserGateway>>().toEqualTypeOf<DoclingParserGateway>();
  expectTypeOf<Extract<keyof DoclingParserGateway,
    "saveSettings" | "downloadModels" | "deleteModels" | "connectService" | "disconnectService"
  >>().toEqualTypeOf<never>();
});
