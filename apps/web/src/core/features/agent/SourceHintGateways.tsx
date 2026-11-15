import {createContext,useContext,type ReactNode} from "react";
import type {SourceDirectoryGateways} from "./sourceDirectory";

const Context=createContext<SourceDirectoryGateways|undefined>(undefined);
/** Scoped read ports let previews resolve the same hints without native I/O. */
export function SourceHintGatewayProvider({gateways,children}:{readonly gateways:SourceDirectoryGateways;readonly children:ReactNode}) {
 return <Context.Provider value={gateways}>{children}</Context.Provider>;
}
export function useSourceHintGateways():SourceDirectoryGateways|undefined {return useContext(Context);}
