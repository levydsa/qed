import { defineCollection, z } from "astro:content";

export const DocumentSchema = z.object({
  id: z.string(),
  title: z.string(),
  date: z.date(),
  department: z.string(),
  university: z.string(),
  tags: z.array(z.string()),
});

const documentCollection = defineCollection({
  type: "content",
  schema: DocumentSchema,
});

export const collections = {
  document: documentCollection,
};
