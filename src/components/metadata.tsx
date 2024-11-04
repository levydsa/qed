import type { z } from "astro/zod";
import type { DocumentSchema } from "../content/config";
import { createContext, useContext } from "react";

export type Metadata = z.infer<typeof DocumentSchema>;

export const MetadataContext = createContext<Metadata | null>(null);

export const MetadataProvider = MetadataContext.Provider;

export const useMetadata = () => {
  const context = useContext(MetadataContext);
  if (!context)
    throw new Error(`useMetadata must be used within a MetadataProvider`);
  return context;
};
