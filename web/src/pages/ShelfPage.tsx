// SPDX-License-Identifier: AGPL-3.0-or-later
// A shelf SLAPorSLOP staff made (crates/slop-api/src/shelves.rs): its apps in staff's order.

import { Link, useParams } from "react-router";

import { api } from "../api";
import { StoreFrame } from "../components/StoreNav";
import { ErrorState, Loading, Shelf } from "../components/ui";
import { useDocumentTitle, useLoad } from "../hooks";

export function ShelfPage() {
  const { slug = "" } = useParams();
  const shelf = useLoad(() => api.shelf(slug), [slug]);
  useDocumentTitle(shelf.data?.title);
  return (
    <StoreFrame sidebar>
      <div className="crumbs">
        <Link to="/">Store</Link> › Picked by SLAPorSLOP
      </div>
      {shelf.error ? (
        <ErrorState error={shelf.error} onRetry={shelf.reload} />
      ) : !shelf.data ? (
        <Loading />
      ) : (
        <>
          <h1 className="page-title">{shelf.data.title}</h1>
          {shelf.data.blurb && <p className="muted">{shelf.data.blurb}</p>}
          <Shelf apps={shelf.data.apps} empty="None of these apps are in the store for you right now." />
        </>
      )}
    </StoreFrame>
  );
}
