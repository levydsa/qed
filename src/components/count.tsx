import {
  createContext,
  useContext,
  useRef,
  type FC,
  type ReactNode,
} from "react";

export const CountContext = createContext<(() => number) | null>(null);

export const CountProvider: FC<{ children: ReactNode }> = ({ children }) => {
  const count = useRef(1);
  const nextCount = () => count.current++;

  return (
    <CountContext.Provider value={nextCount}>{children}</CountContext.Provider>
  );
};

export const useCount = () => {
  const context = useContext(CountContext);
  if (!context)
    throw new Error(`useCount must be used within a CountProvider`);
  return context;
};
