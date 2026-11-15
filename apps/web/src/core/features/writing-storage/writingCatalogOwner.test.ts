import {expect,it,vi} from "vitest";
import {createWritingCatalogOwner} from "./writingCatalogOwner";

it("rejects invalid queries and preparation options before scheduling database work", async () => {
  const transaction = vi.fn(), owner = createWritingCatalogOwner({transaction} as unknown as IDBDatabase);
  for (const query of [{category:"all",view:"recent",limit:0},{category:"all",view:"recent",search:null},{category:"foreign",view:"recent"}])
    await expect(owner.queryWritingNoteCatalog(query as never)).rejects.toMatchObject({code:"invalid_input"});
  for (const options of [null,[],{retryErrors:"true"},{unexpected:true},{scope:"trashed"},{scope:null}])
    await expect(owner.prepareWritingNoteCatalog(options as never)).rejects.toMatchObject({code:"invalid_input"});
  expect(transaction).not.toHaveBeenCalled();
});
