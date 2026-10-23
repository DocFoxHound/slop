// SPDX-License-Identifier: AGPL-3.0-or-later
// How to reach Slop (the Terms' "contact page on Slop"): the published address from
// SLOP_CONTACT_EMAIL, and the quicker routes for reports and account requests.

import { Link } from "react-router";

import { api } from "../api";
import { useDocumentTitle, useLoad } from "../hooks";
import { ErrorState, Loading } from "../components/ui";

export function Contact() {
  useDocumentTitle("Contact");
  const contact = useLoad(() => api.contact(), []);
  return (
    <div className="page narrow doc">
      <h1>Contact</h1>
      {contact.error && <ErrorState error={contact.error} onRetry={contact.reload} />}
      {!contact.data ? (
        !contact.error && <Loading />
      ) : contact.data.email ? (
        <p>
          Email <a href={`mailto:${contact.data.email}`}>{contact.data.email}</a> for legal notices, requests from
          authorities, copyright and other legal claims, security problems, and anything else about Slop.
        </p>
      ) : (
        <p>Slop's contact address isn't published yet.</p>
      )}
      <h2>Quicker routes</h2>
      <ul>
        <li>
          <b>Something on Slop breaks the rules or the law:</b> use <b>Report</b> on its page. Reports are tracked, and
          the <Link to="/legal/content">Content Policy</Link> says how they're handled.
        </li>
        <li>
          <b>Your data:</b> download it or close your account from your account page.
        </li>
      </ul>
    </div>
  );
}
