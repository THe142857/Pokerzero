import * as React from "react";
import * as ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { SnackbarProvider } from "notistack";

//import "../static/css/styles.css";
import UPAC from "./UPAC";
import { ThemeProvider, CssVarsProvider } from "@mui/joy";
import theme from "./theme";

function RootApp() {
  return (
    <SnackbarProvider maxSnack={3}>
      <CssVarsProvider defaultMode="light">
        <ThemeProvider theme={theme}>
          <BrowserRouter>
            <UPAC />
          </BrowserRouter>
        </ThemeProvider>
      </CssVarsProvider>
    </SnackbarProvider>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <RootApp />
);
