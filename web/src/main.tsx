import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";

import { Layout } from "./components/Layout";
import { Admin } from "./pages/Admin";
import { AppPage } from "./pages/AppPage";
import { Guide } from "./pages/Guide";
import { Account, DesktopApprove, SignIn, SignUp } from "./pages/Auth";
import { LegalIndex, LegalPage } from "./pages/Legal";
import { Library } from "./pages/Library";
import { Releases } from "./pages/Releases";
import { Manage } from "./pages/Manage";
import { Download } from "./pages/Download";
import { Browse, CategoryPage, MadeWithPeople, NotFound, Search, TagPage, Upcoming } from "./pages/Listing";
import { AppEditor, Publish } from "./pages/Publish";
import { JobPage, MyJobs } from "./pages/Jobs";
import { People, PersonPage } from "./pages/People";
import { PeopleEdit } from "./pages/PeopleEdit";
import { Sim } from "./pages/Sim";
import { Store } from "./pages/Store";
import { SessionProvider } from "./session";
import "./styles.css";

const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { path: "/", element: <Store /> },
      { path: "/search", element: <Search /> },
      { path: "/upcoming", element: <Upcoming /> },
      { path: "/browse", element: <Browse /> },
      { path: "/browse/:slug", element: <CategoryPage /> },
      { path: "/tag/:slug", element: <TagPage /> },
      { path: "/app/:slug", element: <AppPage /> },
      { path: "/publish", element: <Publish /> },
      { path: "/publish/guide", element: <Guide /> },
      { path: "/publish/new", element: <AppEditor /> },
      { path: "/publish/:slug", element: <AppEditor /> },
      { path: "/publish/:slug/releases", element: <Releases /> },
      { path: "/publish/:slug/manage", element: <Manage /> },
      { path: "/library", element: <Library /> },
      { path: "/download", element: <Download /> },
      { path: "/admin", element: <Admin /> },
      { path: "/desktop-signin/:id", element: <DesktopApprove /> },
      { path: "/signin", element: <SignIn /> },
      { path: "/signup", element: <SignUp /> },
      { path: "/account", element: <Account /> },
      { path: "/legal", element: <LegalIndex /> },
      { path: "/legal/:kind", element: <LegalPage /> },
      { path: "/people", element: <People /> },
      { path: "/people/me", element: <PeopleEdit /> },
      { path: "/people/jobs", element: <MyJobs /> },
      { path: "/people/:handle", element: <PersonPage /> },
      { path: "/jobs/:id", element: <JobPage /> },
      { path: "/made-with-people", element: <MadeWithPeople /> },
      { path: "/sim", element: <Sim /> },
      { path: "*", element: <NotFound /> },
    ],
  },
]);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SessionProvider>
      <RouterProvider router={router} />
    </SessionProvider>
  </StrictMode>,
);
