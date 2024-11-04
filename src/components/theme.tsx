import {
  createContext,
  useContext,
  useEffect,
  useState,
  type FC,
  type ReactNode,
} from "react";

export type Theme = "light" | "dark";

export const ThemeContext = createContext<Theme>("light");

export const ThemeProvider: FC<{ children: ReactNode }> = ({ children }) => {
  const [theme, setTheme] = useState<Theme>("light");

  useEffect(() => {
    const root = document.documentElement;

    const observer = new MutationObserver(() => {
      const dataTheme = root.getAttribute("data-theme") as Theme;
      if (!dataTheme) console.warn("`data-theme` is unset in root");
      setTheme(dataTheme);
    });

    observer.observe(root, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });

    setTheme(root.getAttribute("data-theme") as Theme);
  });

  return (
    <ThemeContext.Provider value={theme}>{children}</ThemeContext.Provider>
  );
};

export const useTheme = () => {
  return useContext(ThemeContext);
};
