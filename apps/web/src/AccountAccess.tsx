import {createContext,useContext} from "react";
export const ExternalAccountContext=createContext(false);
export const useExternalAccount=()=>useContext(ExternalAccountContext);
