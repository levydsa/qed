import {
  createContext,
  useContext,
  useState,
  type FC,
  type ReactNode,
} from "react";

export const TagsContext = createContext<{
  tags: string[];
  addTags: (newTags: string[]) => void;
} | null>(null);

export const TagsProvider: FC<{ initial?: string[], children: ReactNode }> = ({ initial = [], children }) => {
  const [tags, setTags] = useState<string[]>(initial);

  const addTags = (newTags: string[]) => {
    setTags(prev => Array.from(new Set([...prev, ...newTags])))
  };

  return (
    <TagsContext.Provider value={{ tags, addTags }}>
      {children}
    </TagsContext.Provider>
  );
};

export const useTags = () => {
  const context = useContext(TagsContext);
  if (!context)
    throw new Error(`useTags must be used within a MetadataProvider`);
  return context;
};
