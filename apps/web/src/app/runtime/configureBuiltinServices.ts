import type {AppRuntime} from "../../core/runtime/appRuntime";
import {configureLiteratureWorkbench} from "./literature/configureLiteratureWorkbench";
import {configureWritingWorkbench} from "./writing/configureWritingWorkbench";

/** The default product uses the same concrete adapters as selected static
 * products. Full mock runtimes still retain their explicitly supplied owners. */
export function configureBuiltinServices(runtime: AppRuntime): void {
  configureLiteratureWorkbench(runtime);
  configureWritingWorkbench(runtime);
}
