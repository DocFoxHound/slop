// SPDX-License-Identifier: AGPL-3.0-or-later
// Typed client for the Slop API (crates/slop-api). Types mirror the Rust response structs.

import { isDesktop } from "./platform";

export type AppKind = "game" | "app";
export type AppStage = "coming_soon" | "beta" | "released";
export type AppStatus = "draft" | "listed" | "delisted";

export interface TagRef {
  slug: string;
  name: string;
}

export interface Category {
  slug: string;
  name: string;
  description: string;
  tags: TagRef[];
}

/** What an app connects to online: servers Slop can't check. */
export type OnlineKind = "own_servers" | "player_servers" | "peer_to_peer" | "other_services";

export type ContentLabel =
  | "romance"
  | "suggestive"
  | "violence"
  | "horror"
  | "strong_language"
  | "gambling_themes";

/** A slide in the front page's Featured carousel. */
export type FeaturedApp = AppSummary & { blurb: string };

/** The maker's rating breakdown (crates/slop-api/src/votes.rs, METRICS §3, §5). */
export interface RatingGroup extends Tally {
  key: string | null;
}
export interface RatingBreakdown {
  by_version: RatingGroup[];
  by_platform: RatingGroup[];
  impact: Array<{
    version: string;
    to_not_slop: number;
    to_slop: number;
    new_votes: number;
  }>;
}

/** Updates for an installed app (crates/slop-api/src/library.rs). */
export interface AppUpdate {
  slug: string;
  latest: { release_id: string; version: string } | null;
  /** A newer release still being checked: "fetching" (downloading and scanning) or "awaiting_review". */
  pending: { version: string; status: "fetching" | "awaiting_review" } | null;
  /** The official app and its community builds, when there's more than one. */
  builds: Array<{
    slug: string;
    title: string;
    official: boolean;
    platforms: string[];
  }>;
  /** Approved mods for the app, to add in the Slop app. */
  mods?: Array<{
    slug: string;
    title: string;
    author: string;
    note: string | null;
    /** Its newest approved release has code; goes online (sandbox mode suggested for the game). */
    code: boolean;
    online: boolean;
  }>;
  /** Sandbox mode: the maker's declaration, whether it's suggested, and player reports. */
  sandbox_ready?: boolean;
  sandbox_mismatch?: boolean;
  sandbox_suggested?: boolean;
  sandbox_trouble?: boolean;
}

export interface AppSummary {
  slug: string;
  title: string;
  summary: string;
  kind: AppKind;
  stage: AppStage;
  /** Only present when not listed (the maker's own drafts). */
  status?: AppStatus;
  min_age: number;
  /** Themes some players want to know about (DESIGN §5). */
  content_labels: ContentLabel[];
  /** Games: players in Slop Pulse's latest count, if it's recent. */
  players_now: number | null;
  tags: string[];
  /** The maker's store tile (920×430), if they set one. */
  thumbnail_url: string | null;
  /** Current 👍/👎 votes. */
  vote_count: number;
  /** Weighted % Not Slop; null below the minimum number of votes. */
  not_slop_percent: number | null;
  /** Credits a completed, fully paid People job: the "Made with people" badge. */
  made_with_people: boolean;
  /** People who marked it Interested: the demand signal for upcoming apps. */
  interested: number;
  /** A community branch of this app (DESIGN §9). */
  branch_of: string | null;
  is_dead: boolean;
  stage_deadline: string | null;
  updated_at: string;
}

/** Donations (crates/slop-api/src/donations.rs). Amounts are in minor units (cents). */
export interface DonationQuote {
  /** What the donor chose for the maker, before Slop's fee. */
  donation: number;
  /** What the donor pays: always shown first and largest. */
  total: number;
  slop_fee: number;
  /** The processing addition (or, when the donor doesn't cover it, the estimate taken from the maker's share). */
  processing: number;
  maker_receives: number;
  donor_covers_fees: boolean;
}

export interface DonateInfo {
  on: boolean;
  off_reason: string | null;
  currency: string;
  presets: DonationQuote[];
  min_minor: number;
  max_minor: number;
  donor_covers_fees: boolean;
  servers: { statement: string; example_consent: string } | null;
  maker: string;
  cant: string | null;
  simulated: boolean;
  /** A community build or mod: one-off donations give the original's maker a share first. */
  shared: { slug: string; title: string; maker: string; percent: number } | null;
}

export interface MyDonation {
  id: string;
  kind: "one_off" | "monthly";
  app_title: string;
  app_slug: string;
  currency: string;
  donation_minor: number;
  total_minor: number;
  status: "pending" | "paid" | "refunded" | "partially_refunded" | "expired";
  refunded_minor: number;
  share_name: boolean;
  simulated: boolean;
  created_at: string;
  paid_at: string | null;
}

export interface MyServerDonation {
  id: string;
  app_title: string;
  app_slug: string;
  currency: string;
  donation_minor: number;
  total_minor: number;
  status: "pending" | "active" | "cancelled";
  consent_text: string;
  simulated: boolean;
  started_at: string | null;
  next_charge_at: string | null;
  cancelled_at: string | null;
  cancelled_by: "donor" | "servers_shut_down" | "stripe" | null;
}

/** A maker's request to lift a donation hold, and staff's answer (donation_holds.rs). */
export interface DonationHoldRequest {
  id: string;
  message: string;
  created_at: string;
  outcome: "lifted" | "kept" | null;
  answer: string | null;
  answered_at: string | null;
}

/** Staff stopped donations to an app: paused for review, or turned off until fixed. */
export interface DonationHold {
  id: string;
  kind: "review" | "off";
  reason: string;
  ground: string | null;
  placed_at: string;
  lifted_at: string | null;
  lift_note: string | null;
  requests: DonationHoldRequest[];
}

export interface AdminDonationHold extends DonationHold {
  slug: string;
  title: string;
  maker: string;
  placed_by: string;
  lifted_by: string | null;
  active_server_donations: number;
}

export interface DonationSettings {
  donations_on: boolean;
  donation_suggested_minor: number;
  donation_currency: string;
  runs_servers: boolean;
  servers_statement: string | null;
  servers_shut_down_at: string | null;
  account_status: "none" | "pending" | "active" | "restricted";
  active_server_donations: number;
  /** A private app takes no donations until it goes public. */
  private?: boolean;
  payments: "stripe" | "simulated" | "off";
}

export interface DonationSummary {
  currency_totals: Array<{
    currency: string;
    donations: number;
    donated_minor: number;
    slop_fee_minor: number;
    refunded_minor: number;
  }>;
  months: Array<{
    month: string;
    currency: string;
    donations: number;
    donated_minor: number;
  }>;
  active_server_donations: number | null;
  shared: Array<{
    name: string;
    message: string | null;
    currency: string;
    donation_minor: number;
    paid_at: string;
  }>;
  recent: Array<{
    id: string;
    kind: string;
    currency: string;
    total_minor: number;
    status: string;
    refundable: boolean;
    paid_at: string;
  }>;
}

export interface FeeStatement {
  month: string;
  currency: string;
  kind: "donation" | "people_job";
  payments: number;
  received_minor: number;
  slop_fee_minor: number;
  simulated: boolean;
  /** The VAT included in Slop's fee; zero under the reverse charge. */
  vat_minor: number;
}

/** How VAT applies to Slop's fee for this user (LEGAL §3.5): the 5% always includes it. */
export interface VatTreatment {
  kind: "none" | "included" | "reverse_charge";
  rate_bp: number | null;
  country: string;
  vat_number: string | null;
  vat_number_checked: boolean;
  slop_vat_id: string | null;
}

export interface DonationAccount {
  status: string;
  payments: string;
  /** A switch or disconnection waiting out its 72 hours (SECURITY §9). */
  change: {
    kind: "switch" | "disconnect";
    requested_at: string;
    effective_at: string;
    new_account_ready: boolean | null;
  } | null;
}

export type SignInProvider =
  "github" | "google" | "gitlab" | "bitbucket" | "microsoft" | "discord";

export const PROVIDER_LABEL: Record<SignInProvider, string> = {
  github: "GitHub",
  google: "Google",
  gitlab: "GitLab",
  bitbucket: "Bitbucket",
  microsoft: "Microsoft",
  discord: "Discord",
};

export interface LinkedIdentity {
  provider: SignInProvider;
  provider_subject: string;
  login: string | null;
  created_at: string;
  last_sign_in_at: string;
}

/** The account's picture and the linked sign-ins it can come from (crates/slop-api/src/avatars.rs). */
export interface MyPicture {
  /** Slop's copy of the chosen picture; null: the letter. */
  avatar_url: string | null;
  provider: SignInProvider | null;
  subject: string | null;
  fetched_at: string | null;
  choices: Array<{
    provider: SignInProvider;
    subject: string;
    login: string | null;
    /** Slop can show this sign-in's picture now. */
    available: boolean;
    /** How it's kept current: read daily, or updated when they sign in with it. */
    updates: "daily" | "sign_in" | "never";
    /** "Refresh picture" can send them through this sign-in again. */
    can_refresh: boolean;
  }>;
}

export interface CombinePreview {
  provider: SignInProvider;
  name: string;
  apps: number;
  library: number;
  votes: number;
  friends: number;
  teams: number;
  jobs: number;
  chats: number;
  blocked: string | null;
}

export interface RepoOwnership {
  ok: boolean;
  method: "account" | "code";
  message: string;
  link: SignInProvider | null;
  code: string | null;
}

/** Someone in a friends list, request list or block list (crates/slop-api/src/friends.rs). */
export interface FriendPerson {
  id: string;
  name: string;
  since: string;
  /** Their profile picture; null: the letter. */
  avatar_url: string | null;
}

export interface Friends {
  friends: FriendPerson[];
  incoming: FriendPerson[];
  outgoing: FriendPerson[];
}

/** The friend and message buttons on a profile. */
export interface Connection {
  friend: "friends" | "requested" | "incoming" | "none";
  blocked: boolean;
  can_message: boolean;
}

/** A proof-of-work check after repeated unfinished checkouts (crates/slop-api/src/challenge.rs). */
export interface PowChallenge {
  id: string;
  prefix: string;
  difficulty: number;
}

/** A written review on an app (crates/slop-api/src/reviews.rs). */
export interface AppReview {
  id: string;
  /** Set when the author's profile is public. */
  author_id: string | null;
  /** Their name, or "A player" when their profile is private. */
  author: string;
  vote: "not_slop" | "slop";
  body: string;
  created_at: string;
  updated_at: string;
  edited: boolean;
  mine: boolean;
  /** The app maker's reply under it. */
  reply: string | null;
  reply_at: string | null;
  /** The review changed after the maker replied. */
  reply_outdated: boolean;
}

export interface UserProfile {
  id: string;
  name: string;
  /** Their profile picture; null: the letter. */
  avatar_url: string | null;
  joined: string;
  public: boolean;
  is_me: boolean;
  creative_handle: string | null;
  apps: AppSummary[];
  votes: Array<
    AppSummary & {
      my_vote: "not_slop" | "slop";
      review: string | null;
      voted_at: string;
    }
  >;
}

/** Teams (crates/slop-api/src/teams.rs). */
export interface TeamCard {
  slug: string;
  name: string;
  kind: "player" | "official";
  about: string;
  join_policy: "open" | "invite";
  members: number;
  verified: boolean;
}

export interface TeamPage {
  slug: string;
  name: string;
  kind: "player" | "official";
  about: string;
  join_policy: "open" | "invite";
  github_org: string | null;
  verified: boolean;
  checks: Array<{ check: string; ok: boolean; detail: string }>;
  checked_at: string | null;
  dns: { domain: string | null; record: string; value: string } | null;
  role: "owner" | "admin" | "member" | null;
  invited: boolean;
  members: Array<{
    user_id: string;
    name: string;
    role: "owner" | "admin" | "member";
    joined_at: string;
    avatar_url: string | null;
  }>;
  member_count: number;
  apps: AppSummary[];
  picks: Array<AppSummary & { id: string; note: string; by: string }>;
  events: Array<{
    id: string;
    title: string;
    starts_at: string;
    app_slug: string | null;
    app_title: string | null;
    by: string;
    mine: boolean;
  }>;
  shared: AppSummary[];
  hidden: boolean;
  /** Whether you have this team's chat: invite-only player teams, outside the under-18 view. */
  chat: boolean;
}

/** A notification in the bell (crates/slop-api/src/notify.rs). */
export interface Notification {
  id: string;
  category:
    | "jobs"
    | "interested"
    | "moderation"
    | "donations"
    | "shared"
    | "teams"
    | "apps"
    | "chat"
    | "friends"
    | "security"
    | "agents";
  title: string;
  body: string;
  link: string;
  /** Merged unread notifications ("new messages"). */
  count: number;
  created_at: string;
  read: boolean;
}

export interface EmailPrefs {
  /** Whether this server sends email at all. */
  email_on: boolean;
  has_email: boolean;
  categories: Array<{
    category: Notification["category"];
    label: string;
    email: boolean;
  }>;
}

/** A screenshot or video on an app's page (crates/slop-api/src/media.rs). */
export interface GalleryItem {
  id: string;
  kind: "image" | "video";
  /** The picture, or the video's poster; null for a video without one. */
  url: string | null;
  youtube_id: string | null;
  caption: string | null;
  width: number | null;
  height: number | null;
}

/** A picture from the maker's repository, pinned to a commit. */
export interface RepoPicture {
  path: string;
  url: string;
  sha256: string;
  width: number;
  height: number;
  format: string;
}

export type MediaItem =
  | ({ kind: "image"; caption: string | null } & RepoPicture)
  | {
      kind: "video";
      youtube_id: string;
      poster: RepoPicture | null;
      caption: string | null;
    };

/** The order key the server uses for a gallery item. */
export function mediaKey(item: MediaItem): string {
  return item.kind === "image"
    ? `image:${item.path}`
    : `video:${item.youtube_id}`;
}

export interface MediaSet {
  id: string;
  commit_sha: string;
  thumbnail: RepoPicture | null;
  gallery: MediaItem[];
  status: "pending" | "approved" | "rejected" | "superseded";
  status_reason: string | null;
  source: "maker" | "release";
  created_at: string;
  reviewed_at: string | null;
  /** Other makers' apps whose icon looks like this thumbnail (bits differing out of 64). */
  lookalikes?: Array<{ slug: string; title: string; distance: number }>;
}

export interface MediaOverview {
  live: MediaSet | null;
  pending: MediaSet | null;
  rejected: MediaSet | null;
  checked_at: string | null;
  problem: string | null;
  supported: boolean;
  manifest_path: string;
  order: string[];
  result?: "pending" | "unchanged" | "none" | "problem";
}

/** A reviewer's note on something waiting for a decision (crates/slop-api/src/review.rs). */
export interface ReviewNote {
  id: string;
  author: string;
  verdict: "pass" | "concern" | "unsure" | null;
  note: string;
  created_at: string;
}

export interface AdminToken {
  id: string;
  name: string;
  scope: "read" | "review" | "moderate";
  admin: string;
  created_at: string;
  expires_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
}

/** Shelves for an app's page. */
export interface SimilarApps {
  like_this: AppSummary[];
  popular: AppSummary[];
  discover: AppSummary[];
}

export interface AppDetail extends AppSummary {
  description_md: string;
  repo_url: string;
  license_spdx: string;
  platforms: AppPlatform[];
  maker: { display_name: string };
  stage_entered_at: string;
  last_commit_at: string | null;
  tag_details: TagRef[];
  gallery: GalleryItem[];
  /** Creatives credited through People jobs. */
  credits: Credit[];
  is_mine: boolean;
  /** The viewer marked it Interested. */
  /** Private Slop: only its owner and the people it's shared with see it. */
  private: boolean;
  /** The verified official team that publishes it. */
  official_team: { slug: string; name: string } | null;
  /** What it connects to online (servers Slop can't check); empty: it doesn't. */
  online: OnlineKind[];
  online_hosts: string[];
  /** The maker says it keeps local saves only in SLOP_SAVE_DIR and works in sandbox mode. */
  sandbox_ready: boolean;
  /** Accessibility features the maker says it has (ACCESSIBILITY keys). */
  accessibility: string[];
  /** Takes bug reports from players. */
  bug_reports: boolean;
  /** A browser demo running now: where to play it, and until when. */
  demo: { url: string; ends_at: string } | null;
  /** ...but the check of its newest release found saves elsewhere: players aren't told it works. */
  sandbox_mismatch: boolean;
  /** Sandbox mode is suggested: it goes online (servers Slop can't check). */
  sandbox_suggested: boolean;
  /** Enough players reported it doesn't work in sandbox mode. */
  sandbox_trouble: boolean;
  /** The viewer reported that it doesn't work in sandbox mode. */
  sandbox_reported: boolean;
  /** The folder it reads mods from, when its maker named one (DESIGN §9). */
  mods_folder: ModsFolder | null;
  is_interested: boolean;
  /** For a community branch: what it branches from. */
  branch: {
    parent_slug: string;
    parent_title: string;
    parent_maker: string;
    kind: string;
    kind_label: string;
    note: string | null;
  } | null;
  /** For an app: its community builds, ones filling platform gaps first. */
  community: Array<
    AppSummary & {
      author: string;
      branch_kind: string;
      branch_note: string | null;
      platforms: string[];
      adds: string[];
      /** A mod with code, or one that goes online (DESIGN §9). */
      mod_code: boolean;
      mod_online: boolean;
    }
  >;
}

export interface AppPage {
  apps: AppSummary[];
  next_offset: number | null;
}

export interface Me {
  id: string;
  display_name: string;
  email: string | null;
  country_code: string;
  is_admin: boolean;
  share_playtime: boolean;
  /** "Recommendations based on my activity". */
  personalize: boolean;
  /** Confirmed 18+ for People. */
  adult_confirmed: boolean;
  /** Suspended by a moderation decision: can read, appeal, download data, sign out or close the account. */
  suspended: boolean;
  /** Moderation decisions about the user they haven't opened yet. */
  new_decisions: number;
  /** Unread notifications, for the bell. */
  unread_notifications: number;
  /** Votes and reviews shown on the public profile (default on; off in the UK under-18 view). */
  profile_public: boolean;
  /** Whether they have a passkey (optional: a quicker sign-in, never required). */
  has_passkey: boolean;
  /** This browser sent a Global Privacy Control signal. */
  gpc: boolean;
  /** Their profile picture from a linked sign-in, served by Slop; null: the letter. */
  avatar_url: string | null;
}

/** Slop's legal documents (crates/slop-api/src/legal.rs). */
export type LegalKind =
  "terms" | "privacy" | "content" | "publisher" | "people";

export interface LegalVersion {
  version: string;
  summary: string;
  published_at: string;
  effective_at: string;
}

export interface LegalDocument extends LegalVersion {
  kind: LegalKind;
  title: string;
  body_md: string;
  sha256: string;
  in_effect: boolean;
  /** Accepted (an agreement) rather than only read (a policy). */
  accepted: boolean;
  versions: LegalVersion[];
}

/** Where the signed-in user stands with one document (GET /v1/me/legal/{kind}). */
export interface MyLegalDocument {
  kind: LegalKind;
  /** An agreement they accept, rather than a policy they read. */
  accepts: boolean;
  /** The version in effect, if one is. */
  version: string | null;
  /** They've accepted the version in effect (always false for policies). */
  accepted: boolean;
}

export interface LegalIndexEntry {
  kind: LegalKind;
  title: string;
  /** Accepted (an agreement) rather than only read (a policy). */
  accepted: boolean;
  current: LegalVersion | null;
  upcoming: LegalVersion | null;
}

/** A document the signed-in user should hear about. */
export interface LegalNotice {
  kind: LegalKind;
  title: string;
  version: string;
  summary: string;
  effective_at: string;
  in_effect: boolean;
  /** The document's first version: new, not changed. */
  first: boolean;
  action: "accept" | "read";
}

/** Reports and moderation (crates/slop-api/src/moderation.rs). */
export type ReportKind =
  | "app"
  | "review"
  | "profile"
  | "message"
  | "app_review"
  | "team"
  | "chat_message"
  | "bounty";

export interface ReportReason {
  code: string;
  label: string;
  /** Reviewed within 24 hours. */
  urgent: boolean;
}

export interface CopyrightNotice {
  work: string;
  owner: string;
  address: string;
  phone: string;
  good_faith: boolean;
  accurate_under_penalty: boolean;
  signature: string;
}

export interface ReportBody {
  kind: ReportKind;
  subject: string;
  reason: string;
  details: string;
  good_faith: boolean;
  name?: string;
  email?: string;
  copyright?: CopyrightNotice;
}

export interface MyReport {
  reference: string;
  subject_kind: ReportKind;
  subject_label: string;
  reason: string;
  reason_label: string;
  status: "open" | "actioned" | "dismissed";
  outcome: string | null;
  created_at: string;
  decided_at: string | null;
}

export interface ModerationDecision {
  id: string;
  reference: string;
  subject_kind: ReportKind | "account";
  subject_label: string;
  action:
    | "delist_app"
    | "hide_review"
    | "remove_reply"
    | "hide_profile"
    | "hide_team"
    | "hide_bounty"
    | "suspend_account"
    | "warn";
  action_label: string;
  rule: string;
  rule_title: string;
  law: string | null;
  facts: string;
  automated: boolean;
  created_at: string;
  appeal: string | null;
  appealed_at: string | null;
  appeal_status: "pending" | "upheld" | "reversed" | null;
  appeal_reason: string | null;
  appeal_decided_at: string | null;
  reversed_at: string | null;
  can_appeal: boolean;
  appeal_until: string;
  new?: boolean;
}

export interface CounterNotice {
  name: string;
  address: string;
  phone: string;
  mistake_under_penalty: boolean;
  consent_to_jurisdiction: boolean;
  signature: string;
}

export interface ModeratedAccount {
  id: string;
  name: string;
  closed: boolean;
  suspended: boolean;
  decisions: number;
  copyright_strikes: number;
}

export interface QueuedReport {
  id: string;
  reference: string;
  subject_kind: ReportKind;
  subject_label: string;
  link: string | null;
  quoted: string | null;
  account: ModeratedAccount | null;
  reason: string;
  reason_label: string;
  details: string;
  reporter: string;
  copyright: CopyrightNotice | null;
  priority: 0 | 1 | 2;
  due_at: string;
  overdue: boolean;
  created_at: string;
  same_subject: number;
  actions: ModerationDecision["action"][];
  suggested_rule: string | null;
  preserve: boolean;
}

export type QueuedAppeal = ModerationDecision & {
  decided_by: string;
  counter_notice: CounterNotice | null;
  account: ModeratedAccount;
  link: string | null;
};

/** % Not Slop at one point (crates/slop-api/src/votes.rs). */
export interface Tally {
  not_slop: number;
  slop: number;
  not_slop_weight: number;
  slop_weight: number;
  percent: number | null;
  percent_weighted: number | null;
  label: string | null;
}

export interface Ratings extends Tally {
  monthly: Array<Tally & { month: string }>;
  daily: Array<{
    day: string;
    not_slop: number;
    slop: number;
    changed: number;
    withdrawn: number;
  }>;
  /** Days with unusual voting the abuse checks found (YYYY-MM-DD), shaded on the chart. */
  unusual_days?: string[];
  open: boolean;
  min_votes: number;
  mine?: {
    vote: "not_slop" | "slop" | null;
    can_vote: boolean;
    reason: string | null;
    weight: number;
    /** Their written review; hidden when a moderation decision hid it. */
    review?: { body: string; hidden: boolean };
  };
  /** Votes since the first release with work hired through People (null: none hired). */
  since_new_work:
    | (Partial<Tally> & {
        since: string | null;
        version: string | null;
        credits: string[];
      })
    | null;
}

// ---- People (crates/slop-api/src/people) ------------------------------------------------------

export interface Skill {
  slug: string;
  name: string;
}

export type AiUse = "none" | "assisted" | "heavy";
export type Accent =
  "slime" | "ocean" | "grape" | "rose" | "ember" | "sun" | "forest" | "slate";
export type Section =
  "portfolio" | "services" | "reviews" | "worked_on" | "about";
export type Currency = "usd" | "eur" | "gbp";

export interface Service {
  id: string;
  name: string;
  description: string;
  price_minor: number;
  currency: Currency;
  days: number;
  skill: string;
}

export interface CreativeRating {
  not_slop: number;
  slop: number;
  percent: number | null;
  label: string | null;
  min_reviews: number;
}

export interface CreativeCard {
  handle: string;
  name: string;
  kind: "individual" | "studio";
  team_size: number | null;
  headline: string;
  location: string;
  skills: Skill[];
  ai_use: AiUse;
  taking_work: boolean;
  /** Taking work and able to be paid. */
  can_hire: boolean;
  accent: Accent;
  avatar_url: string | null;
  preview: string[];
  from_price: { minor: number; currency: Currency } | null;
  rating: CreativeRating;
  clients: number;
}

export interface Work {
  key: string;
  kind: "image" | "video" | "audio";
  url: string | null;
  width: number | null;
  height: number | null;
  youtube_id: string | null;
  audio_url: string | null;
  audio_format: string | null;
  title: string | null;
  caption: string | null;
  skill: Skill | null;
}

export interface CreativeReview {
  job_id: string;
  client_name: string;
  project: string | null;
  project_slug: string | null;
  work: string;
  skill: Skill;
  vote: "not_slop" | "slop";
  body: string;
  reply: string | null;
  /** A mediator's factual note after a review dispute. */
  mediator_note: string | null;
  /** The creative's dispute about it. */
  dispute: "pending" | "removed" | "noted" | "kept" | null;
  created_at: string;
  replied_at: string | null;
}

export interface CreativePage extends CreativeCard {
  about_md: string;
  ai_note: string;
  trader: boolean;
  turnaround: string;
  sections: Section[];
  links: Array<{ label: string; url: string; domain: string }>;
  banner_url: string | null;
  works: Work[];
  services: Array<Service & { skill_name: string }>;
  reviews: CreativeReview[];
  worked_on: Array<AppSummary & { role: string }>;
  is_me: boolean;
  /** Always live for others; the creative's own `mine.state` for themselves. */
  status: PageState;
  /** Only for the creative themselves: where their page is. */
  mine: PageStatus | null;
  hidden_reason: string | null;
  /** The account was closed: the page is kept, frozen, and takes no new work. */
  closed: boolean;
}

export interface ProfileInput {
  handle: string;
  name: string;
  kind: "individual" | "studio";
  team_size: number | null;
  headline: string;
  about_md: string;
  location: string;
  skills: string[];
  ai_use: AiUse;
  ai_note: string;
  trader: boolean;
  taking_work: boolean;
  turnaround: string;
  services: Service[];
  accent: Accent;
  sections: Section[];
  links: Array<{ label: string; url: string }>;
  confirm_adult?: boolean;
}

export interface MyProfile extends Omit<ProfileInput, "confirm_adult"> {
  portfolio_repo: string | null;
  stripe_status: "none" | "pending" | "active" | "restricted";
  live: boolean;
  hidden_reason: string | null;
  /** stripe, simulated (sandbox/development) or off. */
  payments: "stripe" | "simulated" | "off";
  status: PageStatus;
}

/**
 * Where a creative's People page is (people/mod.rs `PageStatus`). Only portfolios are reviewed:
 * draft (no portfolio sent yet), waiting (the first one waits for review), turned_down (the newest
 * was turned down, none approved yet), live (an approved portfolio; everyone sees the page), or
 * hidden (a moderation decision, or the account is suspended).
 */
export type PageState = "draft" | "waiting" | "turned_down" | "live" | "hidden";

export interface PageStatus {
  state: PageState;
  /** turned_down: the reviewer's reason; live: why a newer portfolio was turned down; hidden: the decision's reason. */
  reason: string | null;
  /** live: a newer portfolio waits for review. */
  update_waiting: boolean;
  waiting_since: string | null;
  /** What went wrong the last time Slop read the portfolio. */
  problem: string | null;
  suspended: boolean;
}

export type PortfolioWork =
  | {
      kind: "image";
      path: string;
      url: string;
      width: number;
      height: number;
      title: string | null;
      caption: string | null;
    }
  | {
      kind: "video";
      youtube_id: string;
      poster: RepoPicture | null;
      title: string | null;
      caption: string | null;
    }
  | {
      kind: "audio";
      audio: { path: string; url: string; format: string };
      poster: RepoPicture | null;
      title: string | null;
      caption: string | null;
    };

export function workKey(w: PortfolioWork): string {
  return w.kind === "image"
    ? `image:${w.path}`
    : w.kind === "video"
      ? `video:${w.youtube_id}`
      : `audio:${w.audio.path}`;
}

export interface PortfolioSet {
  id: string;
  repo_url: string;
  commit_sha: string;
  avatar: RepoPicture | null;
  banner: RepoPicture | null;
  works: PortfolioWork[];
  status: "pending" | "approved" | "rejected" | "superseded";
  status_reason: string | null;
  created_at: string;
  reviewed_at: string | null;
}

export interface PortfolioOverview {
  repo_url: string | null;
  live: PortfolioSet | null;
  pending: PortfolioSet | null;
  rejected: PortfolioSet | null;
  checked_at: string | null;
  problem: string | null;
  order: string[];
  manifest_path: string;
  result?: "pending" | "unchanged" | "none" | "problem";
}

export interface Credit {
  handle: string;
  name: string;
  role: string;
  skill: Skill;
  work: string;
}

export interface Offer {
  title: string;
  scope_md: string;
  price_minor: number;
  currency: Currency;
  payment: "upfront" | "split" | "on_delivery";
  due_date: string;
  /** The client signs off on the direction before the rest is made. */
  sign_off: boolean;
  rights: "transfer" | "licence";
  open_source: boolean;
  credit: boolean;
  ai_use: AiUse;
  refund_policy: string;
}

export type JobStatus =
  | "requested"
  | "offered"
  | "awaiting_payment"
  | "in_progress"
  | "awaiting_sign_off"
  | "delivered"
  | "final_files"
  | "completed"
  | "declined"
  | "cancelled"
  | "withdrawn";

export type JobAction =
  | "offer"
  | "decline"
  | "accept"
  | "pay"
  | "cancel"
  | "send_for_sign_off"
  | "sign_off"
  | "deliver"
  | "accept_delivery"
  | "final_files"
  | "review"
  | "withdraw"
  | "request_change"
  | "propose_change"
  | "offer_change"
  | "decline_change"
  | "withdraw_change"
  | "accept_change"
  | "request_conversation"
  | "open_conversation"
  | "close_conversation"
  | "message"
  | "open_problem";

export interface JobChange {
  id: string;
  number: number;
  status: "requested" | "offered" | "accepted" | "declined" | "withdrawn";
  requested_note: string | null;
  title: string | null;
  scope_md: string | null;
  extra_price_minor: number | null;
  new_due_date: string | null;
  decline_note: string | null;
  text: string | null;
  sha256: string | null;
  created_at: string;
  decided_at: string | null;
}

export interface JobProblem {
  id: string;
  opened_by_me: boolean;
  statement: string;
  reply: string | null;
  status: "open" | "resolved" | "unresolved" | "withdrawn";
  mediator_suggestion: string | null;
  closing_note: string | null;
  created_at: string;
  proposals: Array<{
    id: string;
    mine: boolean;
    kind: "refund" | "cancel" | "redo" | "other";
    amount_minor: number | null;
    note: string;
    status: string;
  }>;
}

/** A side's own copy of a job conversation, as downloaded. */
export interface ConversationCopy {
  job_id: string;
  messages: Array<{
    id: string;
    author_id: string;
    author: string;
    created_at: string;
    body: string;
    sha256: string;
  }>;
}

export interface Job {
  id: string;
  status: JobStatus;
  status_text: string;
  role: "client" | "creative";
  creative: { name: string; handle: string; trader: boolean; ai_use: AiUse };
  client_name: string;
  project: { title: string; slug: string; stage: AppStage };
  skill: Skill;
  request_md: string;
  wanted_by: string | null;
  client_is_business: boolean;
  inquiry: boolean;
  offer: Offer | null;
  offer_count: number;
  summary: string[];
  agreement: {
    text: string;
    sha256: string | null;
    template: string;
    accepted_at: string | null;
  } | null;
  changes: JobChange[];
  due_date: string | null;
  payments: Array<{
    part: "full" | "deposit" | "final" | "change";
    change_number: number | null;
    due_on: "accept" | "delivery";
    amount_minor: number;
    currency: Currency;
    status: "waiting" | "due" | "paid";
    paid_at: string | null;
    simulated: boolean;
  }>;
  paid_minor: number;
  refunded_minor: number;
  creative_receives_minor: number | null;
  withdrawal: { until: string; open: boolean; started_early: boolean } | null;
  withdrawal_days: number | null;
  signed_off_at: string | null;
  final_files_at: string | null;
  auto_complete_at: string | null;
  completed_at: string | null;
  completed_automatically: boolean;
  credited: boolean;
  conversation: {
    open: boolean;
    requested_at: string | null;
    request_note: string | null;
    text_deleted_after: string | null;
    acknowledged: boolean;
    notice: string;
    messages: Array<{
      id: string;
      author: string;
      mine: boolean;
      body: string | null;
      sha256: string;
      created_at: string;
    }>;
  };
  problem: JobProblem | null;
  /** For the creative: how this client has paid before. */
  client_record: {
    completed_jobs: number;
    paid_in_full: number;
    owing: number;
  } | null;
  events: Array<{
    kind: string;
    actor: string | null;
    note: string | null;
    links: string[];
    created_at: string;
  }>;
  actions: JobAction[];
  payment_mode: "stripe" | "simulated" | "off";
  review: {
    vote: "not_slop" | "slop";
    body: string;
    reply: string | null;
    hidden: boolean;
    mediator_note: string | null;
    dispute: {
      reason: string;
      statement: string;
      client_statement: string | null;
      status: string;
      decision_reason: string | null;
    } | null;
  } | null;
  /** What the client acknowledges before requesting and accepting. */
  exact_wording: string;
  created_at: string;
}

export interface JobCard {
  id: string;
  status: JobStatus;
  status_text: string;
  needs_you: boolean;
  other_name: string;
  creative_handle: string;
  project_title: string;
  project_slug: string;
  skill: Skill;
  title: string;
  price: string | null;
  inquiry: boolean;
  conversation_requested: boolean;
  updated_at: string;
}

export interface PeopleQuery {
  skill?: string;
  taking_work?: boolean;
  ai?: "none" | "assisted";
  studios?: boolean;
  q?: string;
  sort?: "shuffle" | "rating" | "clients";
}

export interface ForYouCard extends AppSummary {
  pick: "match" | "discover" | "popular";
  reason: string;
}

export interface ForYou {
  personalized: boolean;
  based_on: string[];
  items: ForYouCard[];
}

export interface SimUser {
  id: string;
  display_name: string;
  personas: string;
  country_code: string;
  library: number;
  votes: number;
}

export interface AdminAppRef {
  slug: string;
  title: string;
  stage: AppStage;
  status: AppStatus;
  repo_url: string;
  maker: string;
}

/** A maker's request to change what players download (crates/slop-api/src/manage.rs). */
export interface AppChange {
  id: string;
  kind: "release" | "repo";
  /** kind = release: the release to hold players at; null means "follow the newest". */
  release_id: string | null;
  release_version: string | null;
  repo_url: string | null;
  note: string | null;
  status: "pending" | "approved" | "rejected" | "cancelled";
  status_reason: string | null;
  created_at: string;
  reviewed_at: string | null;
  automatic: boolean;
}

export interface WaitingRelease {
  id: string;
  version: string;
  status: "fetching" | "awaiting_review";
  source: "maker" | "slop_build" | "github_release";
  created_at: string;
}

export interface ApprovedRelease {
  id: string;
  version: string;
  reviewed_at: string | null;
  automatic: boolean;
}

export interface ManageOverview {
  /** The release players get now. */
  serving: ApprovedRelease | null;
  /** Whether the app is held at `serving` instead of following the newest. */
  held: boolean;
  approved: ApprovedRelease[];
  /** Releases not reviewed yet, newest first: the maker can withdraw one they didn't send. */
  waiting: WaitingRelease[];
  repo_url: string;
  /** Players have the app, so moving it is reviewed by a person. */
  repo_locked: boolean;
  auto_approval_ready: boolean;
  auto_approval_note: string | null;
  changes: AppChange[];
  /** Slop can't read the repository: no new releases until it can (SECURITY §12.2). */
  frozen: Frozen | null;
  /** Private Slop waiting for a reviewer to make it public. */
  public_requested: boolean;
  /** Community builds of Pulse games: null if not needed, else whether the handshake arrived. */
  pulse_required: boolean | null;
}

export type ChangeRequest =
  | { kind: "release"; release_id: string | null; note?: string }
  | { kind: "repo"; repo_url: string; note?: string };

export type StaffRole =
  "owner" | "moderator" | "reviewer" | "support" | "finance" | "security";
export interface StaffActions {
  approvals_required: number;
  actions: Array<{
    id: string;
    kind: string;
    summary: string;
    requested_actor: string;
    approvals_needed: number;
    status: "pending" | "done" | "failed" | "cancelled" | "expired";
    result: string | null;
    requested_at: string;
    approvals: string[];
  }>;
}
export interface StaffList {
  staff: Array<{
    id: string;
    display_name: string;
    roles: StaffRole[];
    suspended: boolean;
    /** AI agents writing to the admins see their name, not only their roles. */
    listed_by_name: boolean;
  }>;
  roles: StaffRole[];
  my_roles: StaffRole[];
  me: string;
  approvals_required: number;
}

/** Kinds of messages AI agents write to the admins (crates/slop-api/src/admin_messages.rs). */
export type AiMessageCategory = "suggestion" | "report" | "other";
/** A thread from an AI agent, as the admin list shows it. */
export interface AiThreadSummary {
  id: string;
  category: AiMessageCategory;
  /** Written by the sender's agent: show as plain text only. */
  untrusted_subject: string;
  status: "open" | "closed";
  /** The kind of token it came with. */
  via: "maker_agent" | "admin_agent";
  /** The staff member it's for; null: every admin. */
  to: { id: string; display_name: string } | null;
  sender: {
    id: string;
    display_name: string;
    apps: number;
    joined_at: string;
    account_age_days: number;
    blocked: boolean;
  };
  message_count: number;
  /** Something new since you last read it. */
  unread: boolean;
  created_at: string;
  last_message_at: string;
  closed_at: string | null;
}
export interface AiThread extends AiThreadSummary {
  note: string;
  messages: Array<{
    id: string;
    from_staff: boolean;
    author_id: string;
    author: string;
    /** Written with an AI agent's token. */
    by_agent: boolean;
    /** The sender's words (plain text only); staff replies are `text`. */
    untrusted_text?: string;
    text?: string;
    /** The token it was sent with, in words; null for staff signed in. */
    sent_with: string | null;
    provenance: { kind: "maker" | "admin"; id: string; name: string; scope: string; created_at: string } | null;
    created_at: string;
  }>;
}

export interface AdminFeatured {
  slug: string;
  title: string;
  blurb: string;
  /** In the store for everyone; a slide that isn't simply doesn't show. */
  listed: boolean;
}

export interface AdminQueue {
  /** Patterns the abuse checks found (abuse.rs). */
  abuse: Array<{
    id: string;
    kind:
      | "vote_burst"
      | "review_bomb"
      | "vote_swing"
      | "lookalike_name"
      | "rate_limited"
      | "malicious_dependency"
      | "bad_link";
    subject_kind: "app" | "account";
    subject_id: string;
    subject_label: string;
    slug: string | null;
    detail: Record<string, unknown>;
    detected_at: string;
  }>;
  /** Live releases a re-scan flagged (rescan.rs): withdraw them or keep them. */
  rescans: Array<{
    release_id: string;
    version: string;
    platform: string;
    file_name: string;
    rescan_result: string | null;
    rescan_report: (Partial<ScanReport> & { detail?: string }) | null;
    rescan_flagged_at: string;
    slug: string;
    title: string;
  }>;
  releases: Array<Release & { app: AdminAppRef; notes: ReviewNote[] }>;
  changes: Array<
    AppChange & {
      app: AdminAppRef;
      players_get: string | null;
      notes: ReviewNote[];
    }
  >;
  media: Array<
    MediaSet & { app: AdminAppRef; live: MediaSet | null; notes: ReviewNote[] }
  >;
  portfolios: Array<{
    handle: string;
    name: string;
    ai_use: AiUse;
    set: PortfolioSet;
    live: PortfolioSet | null;
    notes: ReviewNote[];
  }>;
  problems: Array<{
    id: string;
    job_id: string;
    creative: string;
    client: string;
    title: string;
    statement: string;
    opened_by_creative: boolean;
    reply: string | null;
    mediator_suggestion: string | null;
    proposals: Array<{
      kind: string;
      amount_minor: number | null;
      note: string;
      status: string;
    }>;
    created_at: string;
    notes: ReviewNote[];
  }>;
  review_disputes: Array<{
    job_id: string;
    creative: string;
    creative_handle: string;
    client: string;
    vote: "not_slop" | "slop";
    review: string;
    reason: string;
    reason_text: string;
    statement: string;
    client_statement: string | null;
    created_at: string;
    notes: ReviewNote[];
  }>;
  /** Open reports: urgent first, then by when they're due. */
  reports: QueuedReport[];
  /** Appeals against moderation decisions. */
  appeals: QueuedAppeal[];
  /** Content Policy rules a decision can cite: [code, title]. */
  rules: Array<[string, string]>;
  ready_to_list: Array<
    AdminAppRef & {
      approved_releases: number;
      pictures_pending: boolean;
      going_public: boolean;
      /** Taken out of the store, then reinstated on appeal while it couldn't be listed. */
      after_appeal: boolean;
      /** When it last changed: how long it has waited to be listed. */
      waiting_since: string;
      notes: ReviewNote[];
    }
  >;
  checks: {
    scanning: boolean;
    maker_signatures_required: boolean;
    signing_key: boolean;
    github_app: boolean;
    github_token: boolean;
  };
}

export interface AdminApp extends AdminAppRef {
  releases: Array<{ id: string; version: string; reviewed_at: string | null }>;
  /** Releases still being checked or waiting for review. */
  pending_releases: number;
}

export interface ListingEdit {
  title?: string;
  summary?: string;
  description_md?: string;
  min_age?: number;
  content_labels?: string[];
  reason: string;
}

export interface AdminAppRecord {
  app: {
    id: string;
    slug: string;
    title: string;
    summary: string;
    description_md: string;
    status: AppStatus;
    stage: AppStage;
    visibility: string;
    repo_url: string;
    license_spdx: string;
    min_age: number;
    content_labels: string[];
    platforms: string[];
    maker_id: string;
    maker: string;
    parent: string | null;
    frozen_at: string | null;
    frozen_reason: string | null;
    /** Set when staff froze it by hand. */
    frozen_by: string | null;
    donations_on: boolean;
    donation_account: string | null;
    created_at: string;
    updated_at: string;
  };
  releases: Array<{ id: string; version: string; status: string; created_at: string; reviewed_at: string | null }>;
  donations: Array<{ currency: string; donations: number; donated_minor: number; slop_fee_minor: number; refunded_minor: number }>;
  active_server_donations: number;
  reports: Array<{ id: string; reference: string; reason: string; details: string; status: string; created_at: string }>;
  decisions: Array<{
    id: string;
    reference: string;
    action: string;
    rule: string;
    facts: string;
    decided_by: string;
    appeal_status: string | null;
    created_at: string;
    reversed_at: string | null;
  }>;
  audit: Array<{ id: number; at: string; actor: string; action: string; detail: Record<string, unknown> }>;
}

export interface AdminMoney {
  months: Array<{ month: string; kind: string; currency: string; payments: number; amount_minor: number; slop_fee_minor: number }>;
  recent: Array<{
    id: string;
    kind: string;
    amount_minor: number;
    slop_fee_minor: number;
    currency: string;
    stripe_account_id: string | null;
    stripe_object_id: string | null;
    simulated: boolean;
    created_at: string;
  }>;
  active_server_donations: number;
  open_holds: number;
  hold_requests_waiting: number;
  stripe_events_today: number;
  payments: "stripe" | "simulated" | "off";
}

export interface AdminFoundUser {
  id: string;
  display_name: string;
  handle: string | null;
  created_at: string;
  suspended: boolean;
  closed: boolean;
  is_admin: boolean;
}

export interface AdminUserDetails {
  id: string;
  display_name: string;
  country_code: string | null;
  created_at: string;
  suspended_at: string | null;
  closed_at: string | null;
  is_admin: boolean;
  sign_ins: string[];
  apps: Array<{ slug: string; title: string; status: string; visibility: string }>;
  /** `at` is a Unix time in seconds. */
  reports_about: Array<{ reference: string; about: string; reason: string; status: string; at: number }>;
  decisions: Array<{ id: string; reference: string; action: string; rule: string; about: string; at: number; reversed: boolean }>;
}

/** No new releases (frozen.rs): Slop lost the repository, or staff froze it by hand. */
export interface Frozen {
  reason: string;
  since: string;
  /** When an automatic freeze takes it out of the store; staff freezes never do. */
  until: string;
  by_staff: boolean;
}

/** What staff can fix about an account (admin_tools.rs). */
export interface AdminProfile {
  display_name: string;
  /** Their profile picture; null: the letter. */
  avatar_url: string | null;
  creative: {
    handle: string;
    name: string;
    headline: string;
    about_md: string;
    location: string;
    links: Array<{ label: string; url: string }>;
    hidden_reason: string | null;
  } | null;
  teams: Array<{ slug: string; name: string; kind: string; about: string; role: string; hidden_reason: string | null }>;
}

export interface ProfileEdit {
  display_name?: string;
  creative?: { name?: string; headline?: string; about_md?: string; location?: string; remove_links?: boolean };
  /** Takes their profile picture off (back to the letter). */
  remove_picture?: boolean;
  reason: string;
}

/** Staff-made shelves and the automatic ones (shelves.rs). */
export interface AdminShelves {
  shelves: Array<{
    slug: string;
    title: string;
    blurb: string;
    on_front: boolean;
    apps: Array<{ slug: string; title: string; listed: boolean }>;
  }>;
  built_in: Array<{ key: string; title: string; hidden: boolean }>;
}

export interface StoreShelf {
  slug: string;
  title: string;
  blurb: string;
  apps: AppSummary[];
}

/** A legal document's versions, for staff (legal.rs). */
export interface AdminLegalDocument {
  kind: "terms" | "privacy" | "content" | "publisher" | "people";
  title: string;
  accepted: boolean;
  latest_body_md: string;
  latest_version: string | null;
  /** The newest version as a file for crates/slop-api/legal/, when it came from the panel. */
  file_text: string | null;
  versions: Array<{
    version: string;
    summary: string;
    published_at: string;
    effective_at: string;
    published_by: string | null;
  }>;
}

export interface AuditEntry {
  id: number;
  at: string;
  actor: string;
  action: string;
  subject: string;
  detail: Record<string, unknown>;
}

export interface Country {
  code: string;
  name: string;
  min_age: number;
}

export interface SignupInfo {
  suggested_name: string;
  email: string | null;
  countries: Country[];
}

export interface SessionInfo {
  id: string;
  kind: "web" | "desktop" | "agent";
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  current: boolean;
}

export type Platform =
  "windows" | "mac" | "linux" | "android" | "linux-deb" | "linux-rpm" | "any";
/** What an app's store page says it runs on: the builds every release must include. */
/** "any": a mod, one .zip for every platform. */
export type AppPlatform = "windows" | "linux" | "linux-deb" | "linux-rpm" | "any";
export type FileKind = "archive" | "installer" | "apk";
export type ReleaseStatus =
  | "fetching"
  | "awaiting_review"
  | "approved"
  | "rejected"
  | "failed"
  | "revoked"
  | "withdrawn";

/** Who built a file, proven by the maker's GitHub artifact attestation. */
export interface Provenance {
  source_repo: string;
  commit: string;
  signer_workflow: string;
  built_by_slop_build: boolean;
  runner: string | null;
  run_url: string | null;
}

export interface ReleaseFile {
  platform: Platform;
  kind: FileKind;
  url: string;
  file_name: string;
  exec: string | null;
  args: string[];
  size_bytes: number | null;
  sha256: string | null;
  provenance: Provenance | null;
  provenance_problem: string | null;
  /** Signed by hand with the maker's registered SSH key. */
  maker_key: { fingerprint: string; key_name: string } | null;
  scan_result:
    "passed" | "note" | "on_hold" | "rejected" | "blocked" | "skipped" | null;
  scan_detail: string | null;
  /** Slop's scanner's report: engines, findings, notable capabilities. */
  scan_report: ScanReport | null;
  /** Microsoft Defender's check (Windows builds), from Slop Build or Slop Check. */
  defender: DefenderCheck | null;
  /** A reviewer cleared a finding as a false positive, and why. */
  cleared_reason: string | null;
  /** What Slop is doing with this file right now (crates/slop-api/src/release_progress.rs). */
  check_stage?: FileCheckStage | null;
  check_started_at?: string | null;
  checked_at?: string | null;
  /** waiting, checking, checked, or not_checked (the release stopped before this file). */
  check?: "waiting" | "checking" | "checked" | "not_checked";
}

export type FileCheckStage = "downloading" | "signature" | "scanning";

/** Where a release is in Slop's checks, file by file (crates/slop-api/src/release_progress.rs). */
export interface ReleaseProgress {
  stage: "queued" | "checking_files" | "waiting_for_review" | "approved" | "rejected" | "failed" | "revoked" | "withdrawn";
  files_total: number;
  files_checked: number;
  current?: {
    number: number;
    platform: Platform;
    label: string;
    step: FileCheckStage;
    step_label: string;
    started_at: string;
  };
  /** When Slop started checking the first file. */
  started_at: string | null;
  /** About how long one file takes on this server; null when there's nothing to go on. */
  seconds_per_file: number | null;
  remaining_seconds: number | null;
  /** A sentence for the maker (and their AI agent). */
  summary: string;
}

export interface ScanReport {
  result: string;
  detail: string;
  engines: Record<string, string>;
  findings: Array<{ engine: string; file: string; name: string }>;
  capabilities: Array<{ file: string; capability: string; namespace: string }>;
}

export interface DefenderCheck {
  sha256: string;
  engine: string;
  signatures: string;
  signatures_updated: string | null;
  detections: Array<{ file: string; threat: string }>;
  signed_by: string | null;
}

export interface SourceChecks {
  secrets: Array<{ file: string; line: number | null; rule: string }>;
  vulnerabilities: Array<{
    package: string;
    version: string;
    ecosystem: string;
    id: string;
    summary: string;
  }>;
  tools: Record<string, string>;
  checked: boolean;
}

/** A pattern Slop's source check found: what, how often, and where (`path:line`). */
export interface SourceHit {
  rule: string;
  label: string;
  count: number;
  examples: string[];
}

/** Slop's own look at the source at the release commit (crates/slop-api/src/source_scan.rs). */
export interface RepoScan {
  status: "checked" | "not_checked" | "failed";
  detail?: string;
  truncated?: boolean;
  licence?: {
    declared: string;
    files: Array<{ path: string; spdx: string | null }>;
    headers: SourceHit[];
    problems: string[];
  };
  lockfiles?: string[];
  packages?: number;
  monetization?: SourceHit[];
  /** Loading code at run time, self-updaters, unsafe data formats. */
  code_loading?: SourceHit[];
  /** Networking code: it goes online. */
  network?: SourceHit[];
  /** Saves written to fixed places in the user's folders instead of SLOP_SAVE_DIR. */
  saves?: SourceHit[];
  /**
   * Where local saves go, as far as the source shows: SLOP_SAVE_DIR (`slop`), an engine folder
   * the Slop app points into the save folder (`engine`, Godot's user://), fixed per-user folders
   * (`elsewhere`, see `saves`), or no local saves found (`none`).
   */
  save_folder?: "slop" | "engine" | "elsewhere" | "none";
  /** The store page says it works in sandbox mode, but the code saves elsewhere. */
  sandbox_mismatch?: boolean;
  /** Code in the repository, by language (shown for mods). */
  code_files?: SourceHit[];
  /** A mod with code: it gets a full read. */
  mod_code?: boolean;
  /** A mod with networking code: flagged, and sandbox mode is suggested for its game. */
  mod_online?: boolean;
  /** Pay-to-win designs a person should look at. */
  design?: SourceHit[];
  /** Slop Pulse in the code. */
  pulse?: "none" | "start_only" | "heartbeat";
  /** A community build of a game that counts players, without Pulse. */
  pulse_missing?: boolean;
  /** It goes online, but the store page didn't say so when this release was checked. */
  online_undeclared?: boolean;
}

/** An advisory OSV lists for one of the release's packages (crates/slop-api/src/dep_watch.rs). */
export interface Advisory {
  vuln_id: string;
  ecosystem: string;
  package: string;
  version: string;
  summary: string;
  fixed: string | null;
  malicious: boolean;
  known_at_build: boolean;
  found_at: string;
}

export interface Release {
  id: string;
  version: string;
  commit_sha: string;
  status: ReleaseStatus;
  status_reason?: string;
  commit_verified: boolean;
  created_at: string;
  /** Who sent it: the maker, Slop Build, or Slop's GitHub release watcher. */
  source: "maker" | "slop_build" | "github_release";
  /** Approved by Slop as an update, without waiting for a person. */
  auto_approved: boolean;
  /** Its rating (AGE-7). */
  rating?: ReleaseRating | null;
  /** Shared runtimes it needs (Visual C++, .NET Desktop, DirectX). */
  runtimes?: string[];
  /** What changed, for players (Markdown). */
  notes?: string | null;
  files: ReleaseFile[];
  /** Slop Build's source checks. */
  source_checks: SourceChecks | null;
  /** Slop's own source check: licence, monetization notes, packages (makers and reviewers). */
  repo_scan: RepoScan | null;
  /** Advisories for its packages, from the daily watch (makers and reviewers). */
  advisories: Advisory[];
  /** Where it is in Slop's checks, file by file. */
  progress?: ReleaseProgress;
}

/** A token for a maker's or creative's AI agent (crates/slop-api/src/maker_agents.rs). */
export interface AgentToken {
  id: string;
  name: string;
  scope: "read" | "publish";
  /** App addresses; empty: all your apps. */
  apps: string[];
  people: boolean;
  created_at: string;
  expires_at: string;
  last_used_at: string;
  /** Past its expiry: it no longer works (listed for a week after). */
  expired: boolean;
}

/** A community build's fix offered to the original (crates/slop-api/src/offers.rs). */
export interface UpstreamOffer {
  id: string;
  /** received: offered to this app; sent: offered by this community build. */
  direction: "received" | "sent";
  branch_slug: string;
  branch_title: string;
  original_slug: string;
  original_title: string;
  offered_by: string;
  pull_request_url: string;
  message: string;
  status: "open" | "accepted" | "declined" | "withdrawn";
  answer: string | null;
  created_at: string;
  answered_at: string | null;
}

/** A fix for a problem Slop's checks found (crates/slop-api/src/fixes.rs). */
export interface Fix {
  kind: string;
  severity: "must" | "should";
  problem: string;
  instruction: string;
  guide_section?: string;
}

/** One of the repository's GitHub releases, for "Pick a release". */
export interface GithubRelease {
  tag: string;
  name: string | null;
  version: string;
  published_at: string | null;
  prerelease: boolean;
  html_url: string;
  /** Already sent to Slop. */
  submitted: boolean;
  assets: Array<{
    name: string;
    url: string;
    size: number;
    platform: Platform | null;
    kind: FileKind | null;
  }>;
}

export interface UpdateSettings {
  /** Slop's GitHub App on this repository; absent when Slop has no App set up. */
  github_app?: { installed: boolean; install_url: string } | null;
  watch_releases: boolean;
  auto_update: boolean;
  auto_approval_ready: boolean;
  auto_approval_note: string | null;
  watch_note: string | null;
  watch_last_tag: string | null;
  watch_checked_at: string | null;
  watch_problem: string | null;
}

export interface NewRelease {
  version: string;
  commit_sha: string;
  /** What changed, for players (Markdown). */
  notes?: string;
  files: Array<{
    platform: Platform;
    kind: FileKind;
    url: string;
    exec?: string;
    args?: string[];
  }>;
  /** This release's rating (AGE-7); the store page's when left out. */
  rating?: ReleaseRating;
  /** Shared runtimes it needs; the last approved release's when left out. */
  runtimes?: string[];
}

/** A release's age rating and content labels. */
export interface ReleaseRating {
  min_age: number;
  content_labels: ContentLabel[];
}

export interface LibraryEntry extends AppSummary {
  added_at: string;
}

export interface NewApp {
  slug: string;
  title: string;
  summary: string;
  description_md: string;
  kind: AppKind;
  stage: AppStage;
  repo_url: string;
  license_spdx: string;
  min_age: number;
  content_labels?: ContentLabel[];
  tags: string[];
  platforms: AppPlatform[];
  /** "private": Private Slop, only for you and the people you share it with. */
  visibility?: "public" | "private";
  /** A private repository outside GitHub: a read-only token for it. */
  repo_token?: RepoTokenInput;
  /** What it connects to online; empty: it doesn't play online. */
  online?: OnlineKind[];
  /** The maker's servers by name (play.example.com, play.example.com:7777). */
  online_hosts?: string[];
  /** Works in sandbox mode: local saves only in SLOP_SAVE_DIR. */
  sandbox_ready?: boolean;
  /** The folder it reads mods from; null: it reads SLOP_MODS, or has no mods. */
  mods_folder?: ModsFolder | null;
  /** Accessibility features it has (ACCESSIBILITY keys). */
  accessibility?: string[];
  /** Takes bug reports from players. */
  bug_reports?: boolean;
}

/** Where a game reads mods from: Slop keeps only approved mods there (DESIGN §9). */
export interface ModsFolder {
  /** Its installed files, or its save folder. */
  base: "game" | "saves";
  /** Relative, with `/`: `mods`, `Content/Paks/~mods`. */
  path: string;
  /** Each mod in its own folder, or every mod's files straight in the folder. */
  layout: "folder" | "flat";
}

export type RepoTokenKind = "gitlab" | "bitbucket" | "gitea";
export interface RepoTokenInput {
  kind: RepoTokenKind;
  token: string;
}
export interface RepoTokenStatus {
  saved: boolean;
  kind: RepoTokenKind | null;
  saved_at: string | null;
  last_used_at: string | null;
  /** The server can keep tokens (SLOP_REPO_TOKEN_KEY is set). */
  available: boolean;
  github: boolean;
}

export type AppChanges = Partial<
  Omit<NewApp, "slug" | "kind" | "stage" | "visibility" | "repo_token">
>;

export interface AppQuery {
  q?: string;
  tag?: string;
  category?: string;
  stage?: AppStage;
  kind?: AppKind;
  sort?:
    | "recent"
    | "title"
    | "relevance"
    | "trending"
    | "anticipated"
    | "rating"
    | "popular"
    | "reviewed"
    | "not_slop_now"
    | "fresh_slop"
    | "barely_seen";
  made_with_people?: boolean;
  /** Has every one of these accessibility features (comma-separated). */
  accessibility?: string;
  /** Runs on this platform, directly or through a community build. */
  platform?: "windows" | "linux";
  /** Only upcoming (Coming soon and Beta) or only released. */
  upcoming?: boolean;
  /** At least this weighted % Not Slop. */
  min_not_slop?: number;
  /** Community branches instead of apps. */
  community?: boolean;
  limit?: number;
  offset?: number;
}

/** Accessibility features a maker can declare (crates/slop-api/src/label.rs FEATURES). */
export const ACCESSIBILITY: Array<[string, string]> = [
  ["remap_controls", "Remappable controls"],
  ["colorblind", "Colourblind modes"],
  ["subtitles", "Subtitles or captions"],
  ["text_size", "Adjustable text size"],
  ["one_handed", "Playable one-handed"],
  ["difficulty", "Difficulty or assist options"],
  ["no_flashing", "No flashing lights"],
  ["screen_reader", "Works with a screen reader"],
  ["controller", "Full controller support"],
];

/** A player's bug report (crates/slop-api/src/bug_reports.rs). */
export interface BugReport {
  id: string;
  title: string;
  status: "open" | "fixed" | "wont_fix" | "duplicate";
  version: string | null;
  platform: string | null;
  me_too: number;
  mine: boolean;
  i_have_it_too: boolean;
  created_at: string;
  /** For the maker and the player who filed it. */
  body?: string;
  log?: string;
  maker_note?: string;
  /** For the maker. */
  reporter?: string;
}

export interface ReleaseFeedback {
  version: string;
  approved_at: string;
  votes: number;
  not_slop_percent: number | null;
  reviews_up: number;
  reviews_down: number;
  praised: Array<[string, number]>;
  criticised: Array<[string, number]>;
  bug_reports: number;
}

/** A fix bounty (crates/slop-api/src/bounties.rs). */
export interface Bounty {
  id: string;
  app_slug: string;
  app_title: string;
  title: string;
  body: string;
  currency: string;
  status: "open" | "claimed" | "paying" | "paid" | "expired" | "hidden";
  backers: number;
  pledged_minor: number;
  created_at: string;
  expires_at: string;
}

export interface BountyDetail extends Bounty {
  claims: Array<{
    id: string;
    builder: string;
    app_slug: string;
    app_title: string;
    version: string;
    note: string;
    status: "pending" | "accepted" | "declined" | "withdrawn";
    decided_by: "maker" | "backers" | null;
    created_at: string;
    accept_minor: number;
    reject_minor: number;
    /** The viewer delivered it: they may withdraw it while it waits. */
    mine: boolean;
  }>;
  my_pledge: { amount_minor: number; status: string; failure: string | null } | null;
  is_app_maker: boolean;
  can_vote: boolean;
  my_vote: boolean | null;
  payments: "stripe" | "simulated" | "off";
}

/** "What this app does" (label.rs). */
export interface AppLabel {
  items: Array<{ key: string; title: string; detail?: string; checked: boolean; tone: "good" | "info" | "note" }>;
  accessibility: Array<{ key: string; name: string; confirmed: number; disputed: number; mine: boolean | null }>;
  can_report: boolean;
  version: string | null;
}

/** docs/standard/file-verification.md */
export interface FileVerification {
  schema: number;
  sha256: string;
  issued_at: string;
  expires_at: string;
  matches: Array<{
    app_slug: string;
    app_title: string;
    maker: string;
    release_id: string;
    version: string;
    platform: string;
    status: "approved" | "revoked";
    approved_at: string;
    revoked_at?: string;
    scan: string;
    defender?: "clean" | "detections";
    page_url: string;
  }>;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly field?: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/** The website calls its own origin; in development Vite proxies /v1 to the API. */
export const API_BASE = "";

export function query(params: object): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== "")
      search.set(key, String(value));
  }
  const s = search.toString();
  return s ? `?${s}` : "";
}

/** In the desktop app, requests go through Rust, which holds the user's tokens. */
async function desktopRequest(
  method: string,
  path: string,
  body?: unknown,
): Promise<{ status: number; body: unknown }> {
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return await invoke<{ status: number; body: unknown }>("api_request", {
      methodName: method,
      path,
      body: body ?? null,
    });
  } catch (err) {
    const e = err as { code?: string; message?: string };
    throw new ApiError(
      0,
      e.code ?? "network",
      e.message ?? "Can't reach Slop. Check your connection and try again.",
    );
  }
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  if (isDesktop) {
    const res = await desktopRequest(method, path, body);
    if (res.status >= 200 && res.status < 300) return res.body as T;
    const err = (
      res.body as {
        error?: { code?: string; message?: string; field?: string };
      } | null
    )?.error;
    throw new ApiError(
      res.status,
      err?.code ?? "http",
      err?.message ?? `Request failed (${res.status})`,
      err?.field,
    );
  }

  const init: RequestInit = {
    method,
    headers: { Accept: "application/json" },
    // Cookies only travel same-origin; the packaged desktop app reads the API without them.
    credentials: "same-origin",
  };
  if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }

  let res: Response;
  try {
    res = await fetch(API_BASE + path, init);
  } catch {
    throw new ApiError(
      0,
      "network",
      "Can't reach Slop. Check your connection and try again.",
    );
  }
  if (res.status === 204) return undefined as T;

  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const err = (
      data as {
        error?: { code?: string; message?: string; field?: string };
      } | null
    )?.error;
    throw new ApiError(
      res.status,
      err?.code ?? "http",
      err?.message ?? `Request failed (${res.status})`,
      err?.field,
    );
  }
  return data as T;
}

/** A text response (job records). The desktop app returns JSON bodies only, so it gets text back as a string too. */
async function requestText(method: string, path: string): Promise<string> {
  if (isDesktop) {
    const res = await desktopRequest(method, path);
    if (res.status >= 200 && res.status < 300)
      return typeof res.body === "string" ? res.body : JSON.stringify(res.body);
    throw new ApiError(res.status, "http", `Request failed (${res.status})`);
  }
  const res = await fetch(API_BASE + path, {
    method,
    credentials: "same-origin",
  }).catch(() => {
    throw new ApiError(
      0,
      "network",
      "Can't reach Slop. Check your connection and try again.",
    );
  });
  if (!res.ok)
    throw new ApiError(res.status, "http", `Request failed (${res.status})`);
  return res.text();
}

export const api = {
  categories: () => request<Category[]>("GET", "/v1/categories"),
  apps: (q: AppQuery = {}) => request<AppPage>("GET", `/v1/apps${query(q)}`),
  label: (slug: string) => request<AppLabel>("GET", `/v1/apps/${encodeURIComponent(slug)}/label`),
  reportAccessibility: (slug: string, feature: string, works: boolean | null) =>
    works === null
      ? request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/accessibility/${encodeURIComponent(feature)}`)
      : request<void>("PUT", `/v1/apps/${encodeURIComponent(slug)}/accessibility/${encodeURIComponent(feature)}`, {
          works,
        }),
  demo: {
    status: (slug: string) =>
      request<{
        available: boolean;
        eligible: boolean;
        eligible_until: string | null;
        days: number;
        play_url: string | null;
        demo: {
          status: "fetching" | "live" | "failed" | "ended";
          detail: string | null;
          scan_result: string | null;
          size_bytes: number | null;
          plays: number;
          ends_at: string;
        } | null;
      }>("GET", `/v1/apps/${encodeURIComponent(slug)}/demo`),
    start: (slug: string, url: string) => request<void>("POST", `/v1/apps/${encodeURIComponent(slug)}/demo`, { url }),
    end: (slug: string) => request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/demo`),
  },
  bugReports: {
    list: (slug: string, status?: string) =>
      request<{ reports: BugReport[]; is_maker: boolean; can_report: boolean; accepting: boolean }>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/bug-reports${query({ status })}`,
      ),
    file: (
      slug: string,
      body: { title: string; body: string; version?: string; platform?: string; log?: string },
    ) => request<BugReport>("POST", `/v1/apps/${encodeURIComponent(slug)}/bug-reports`, body),
    update: (id: string, change: { status?: BugReport["status"]; maker_note?: string }) =>
      request<BugReport>("PATCH", `/v1/bug-reports/${encodeURIComponent(id)}`, change),
    meToo: (id: string, on: boolean) =>
      request<void>(on ? "POST" : "DELETE", `/v1/bug-reports/${encodeURIComponent(id)}/me-too`),
    feedback: (slug: string) =>
      request<ReleaseFeedback[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/release-feedback`),
  },
  bounties: {
    forApp: (slug: string) => request<Bounty[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/bounties`),
    all: () => request<Bounty[]>("GET", "/v1/bounties"),
    get: (id: string) => request<BountyDetail>("GET", `/v1/bounties/${encodeURIComponent(id)}`),
    open: (slug: string, body: { title: string; body: string; currency: string }) =>
      request<Bounty>("POST", `/v1/apps/${encodeURIComponent(slug)}/bounties`, body),
    pledge: (
      id: string,
      amount_minor: number,
      extra: { confirm_adult?: boolean; challenge?: { id: string; nonce: string } } = {},
    ) =>
      request<{ url: string | null; status: string }>("POST", `/v1/bounties/${encodeURIComponent(id)}/pledge`, {
        amount_minor,
        ...extra,
      }),
    checkPledge: (id: string) => request<void>("POST", `/v1/bounties/${encodeURIComponent(id)}/pledge/check`),
    withdraw: (id: string) => request<void>("DELETE", `/v1/bounties/${encodeURIComponent(id)}/pledge`),
    claim: (id: string, app_slug: string, note: string) =>
      request<void>("POST", `/v1/bounties/${encodeURIComponent(id)}/claims`, { app_slug, note }),
    answer: (id: string, claim: string, decision: "accepted" | "declined") =>
      request<void>("POST", `/v1/bounties/${encodeURIComponent(id)}/claims/${encodeURIComponent(claim)}/answer`, {
        decision,
      }),
    vote: (id: string, claim: string, accept: boolean) =>
      request<void>("POST", `/v1/bounties/${encodeURIComponent(id)}/claims/${encodeURIComponent(claim)}/vote`, {
        accept,
      }),
    withdrawClaim: (id: string, claim: string) =>
      request<void>("POST", `/v1/bounties/${encodeURIComponent(id)}/claims/${encodeURIComponent(claim)}/withdraw`),
  },
  /** Staff-made front-page shelves, and the automatic ones staff hid. */
  shelves: () => request<{ shelves: StoreShelf[]; hidden: string[] }>("GET", "/v1/shelves"),
  shelf: (slug: string) => request<StoreShelf>("GET", `/v1/shelves/staff/${encodeURIComponent(slug)}`),
  glowUps: () => request<Array<AppSummary & { before: number; after: number }>>("GET", "/v1/shelves/glow-ups"),
  app: (slug: string) =>
    request<AppDetail>("GET", `/v1/apps/${encodeURIComponent(slug)}`),
  createApp: (app: NewApp) => request<AppDetail>("POST", "/v1/apps", app),
  updateApp: (slug: string, changes: AppChanges) =>
    request<AppDetail>(
      "PATCH",
      `/v1/apps/${encodeURIComponent(slug)}`,
      changes,
    ),
  myApps: () => request<AppSummary[]>("GET", "/v1/me/apps"),
  similar: (slug: string) =>
    request<SimilarApps>("GET", `/v1/apps/${encodeURIComponent(slug)}/similar`),
  /** Pictures from the repository's .slop/media.json (crates/slop-api/src/media.rs). */
  media: (slug: string) =>
    request<MediaOverview>("GET", `/v1/apps/${encodeURIComponent(slug)}/media`),
  checkMedia: (slug: string) =>
    request<MediaOverview>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/media/check`,
    ),
  setMediaOrder: (slug: string, keys: string[]) =>
    request<MediaOverview>(
      "PUT",
      `/v1/apps/${encodeURIComponent(slug)}/media/order`,
      { keys },
    ),
  /** Whether a store address is free, with a free one like it when it isn't (catalog.rs). */
  addressAvailable: (slug: string) =>
    request<{ slug: string; available: boolean; suggestion: string | null }>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/available`,
    ),
  /** The licence GitHub finds in a repository, for the Publish form. */
  detectLicense: (repoUrl: string) =>
    request<{ spdx: string | null; name: string | null; note: string | null }>(
      "GET",
      `/v1/repos/license${query({ url: repoUrl })}`,
    ),
  releases: (slug: string) =>
    request<Release[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/releases`),
  githubReleases: (slug: string) =>
    request<GithubRelease[]>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/github-releases`,
    ),
  submitGithubRelease: (
    slug: string,
    tag: string,
    files: NewRelease["files"],
    rating?: ReleaseRating,
    runtimes?: string[],
    notes?: string,
  ) =>
    request<Release>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/releases/from-github`,
      { tag, files, rating, runtimes, notes },
    ),
  /** The approved shared runtimes a release can need. */
  runtimes: () => request<Array<{ id: string; name: string; vendor: string }>>("GET", "/v1/runtimes"),
  updateSettings: (slug: string) =>
    request<UpdateSettings>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/update-settings`,
    ),
  changeUpdateSettings: (
    slug: string,
    change: Partial<Pick<UpdateSettings, "watch_releases" | "auto_update">>,
  ) =>
    request<UpdateSettings>(
      "PUT",
      `/v1/apps/${encodeURIComponent(slug)}/update-settings`,
      change,
    ),
  submitRelease: (slug: string, release: NewRelease) =>
    request<Release>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/releases`,
      release,
    ),
  /** Whether an approved, signed build exists (the manifest itself is checked in Rust). */
  /** For installed apps: the release players get, a newer one being checked, and other builds. */
  libraryUpdates: (slugs: string[]) =>
    request<AppUpdate[]>(
      "GET",
      `/v1/library/updates?apps=${encodeURIComponent(slugs.join(","))}`,
    ),
  hasBuild: (slug: string) =>
    request<unknown>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/manifest`,
    ).then(
      () => true,
      (err: unknown) => {
        if (err instanceof ApiError && err.status === 404) return false;
        throw err;
      },
    ),
  slopBuildSetup: (slug: string) =>
    request<{
      supported: boolean;
      project_type:
        | "rust"
        | "go"
        | "dotnet"
        | "godot"
        | "godot-dotnet"
        | "unity"
        | "web"
        | "electron"
        | "tauri"
        | "python"
        | "custom";
      detected: string | null;
      note: string | null;
      file_path: string;
      workflow: string;
      create_url: string | null;
      check_file_path: string;
      check_workflow: string;
      check_create_url: string | null;
      build_now_url: string | null;
    }>("GET", `/v1/apps/${encodeURIComponent(slug)}/slop-build`),
  manage: (slug: string) =>
    request<ManageOverview>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/manage`,
    ),
  /** "Check now" for a frozen app's repository; the freeze if it's still frozen. */
  repoCheck: (slug: string) =>
    request<ManageOverview["frozen"]>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/repo-check`,
    ),
  makePublic: (slug: string) =>
    request<void>("POST", `/v1/apps/${encodeURIComponent(slug)}/make-public`),
  cancelMakePublic: (slug: string) =>
    request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/make-public`),
  requestChange: (slug: string, change: ChangeRequest) =>
    request<AppChange>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/changes`,
      change,
    ),
  /** Withdraws a release that hasn't been reviewed yet. */
  withdrawRelease: (slug: string, id: string) =>
    request<Release>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/releases/${encodeURIComponent(id)}/withdraw`,
    ),
  cancelChange: (slug: string, id: string) =>
    request<AppChange>(
      "POST",
      `/v1/apps/${encodeURIComponent(slug)}/changes/${encodeURIComponent(id)}/cancel`,
    ),
  library: () => request<LibraryEntry[]>("GET", "/v1/me/library"),
  /** What Slop knows about a file, by its SHA-256 (docs/standard/file-verification.md). */
  verifyFile: async (sha256: string) => {
    const signed = await request<{ payload: string; signature: string; key_id: string }>(
      "GET",
      `/v1/verify/${encodeURIComponent(sha256)}`,
    );
    return JSON.parse(signed.payload) as FileVerification;
  },
  /** Where Slop's source code is, at the version running (the AGPL's offer). */
  source: () =>
    request<{ licence: string; url: string | null; commit: string | null; version_url: string | null }>(
      "GET",
      "/v1/source",
    ),
  /** Slop's published contact address (contact page); null until the server sets one. */
  contact: () => request<{ email: string | null }>("GET", "/v1/contact"),
  /** The yearly transparency report (counts only). */
  transparency: (year: number) => request<TransparencyReport>("GET", `/v1/transparency/${year}`),
  admin: {
    queue: () => request<AdminQueue>("GET", "/v1/admin/queue"),
    /** The law-enforcement request log (legal_requests.rs); staff with sensitive rights. */
    legalRequests: () => request<LegalRequest[]>("GET", "/v1/admin/legal-requests"),
    logLegalRequest: (r: NewLegalRequest) =>
      request<{ id: string; reference: string; deadline: string | null }>("POST", "/v1/admin/legal-requests", r),
    /** A terrorist content removal order, acted on in one step (1-hour deadline). */
    removalOrder: (o: {
      received_at: string;
      country: string;
      authority: string;
      order: string;
      subject_kind: "app" | "app_review" | "profile" | "team";
      subject: string;
    }) => request<{ reference: string; minutes: number; label: string }>("POST", "/v1/admin/removal-orders", o),
    answerLegalRequest: (id: string, a: { status: string; response: string; user_notice: string }) =>
      request<{ reference: string }>("POST", `/v1/admin/legal-requests/${encodeURIComponent(id)}/answer`, a),
    /** What needs an admin right now (ops.rs): stale backups, failed deploys, overdue reports… */
    ops: () =>
      request<
        Array<{
          key: string;
          title: string;
          detail: string;
          link: string;
          first_seen: string;
        }>
      >("GET", "/v1/admin/ops"),
    /** Sensitive actions waiting for (or done after) enough staff approvals (staff.rs). */
    actions: (all = false) =>
      request<StaffActions>(
        "GET",
        `/v1/admin/actions${all ? "?status=all" : ""}`,
      ),
    approveAction: (id: string) =>
      request<unknown>("POST", `/v1/admin/actions/${id}/approve`),
    cancelAction: (id: string) =>
      request<void>("POST", `/v1/admin/actions/${id}/cancel`),
    staff: () => request<StaffList>("GET", "/v1/admin/staff"),
    changeRole: (userId: string, role: StaffRole, grant: boolean) =>
      request<unknown>(
        "POST",
        `/v1/admin/staff/${encodeURIComponent(userId)}/roles`,
        { role, grant },
      ),
    removeStaff: (userId: string, reason: string) =>
      request<unknown>("POST", `/v1/admin/staff/${encodeURIComponent(userId)}/remove`, { reason }),
    /** What staff can fix about an account, and the fix (the person is told why). */
    profile: (id: string) => request<AdminProfile>("GET", `/v1/admin/users/${encodeURIComponent(id)}/profile`),
    editProfile: (id: string, body: ProfileEdit) =>
      request<{ changed: string[] }>("PATCH", `/v1/admin/users/${encodeURIComponent(id)}/profile`, body),
    editTeam: (slug: string, body: { name?: string; about?: string; reason: string }) =>
      request<{ changed: string[] }>("PATCH", `/v1/admin/teams/${encodeURIComponent(slug)}`, body),
    /** No new releases until staff lift it; the app stays in the store. */
    freeze: (slug: string, reason: string) =>
      request<Frozen | null>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/freeze`, { reason }),
    /** Lifts a staff freeze, or checks the repository again for an automatic one. */
    unfreeze: (slug: string, note: string) =>
      request<Frozen | null>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/unfreeze`, { note }),
    shelves: () => request<AdminShelves>("GET", "/v1/admin/shelves"),
    createShelf: (b: { slug: string; title: string; blurb: string; on_front: boolean; apps: string[] }) =>
      request<AdminShelves>("POST", "/v1/admin/shelves", b),
    updateShelf: (slug: string, b: { title: string; blurb: string; on_front: boolean; apps: string[] }) =>
      request<AdminShelves>("PUT", `/v1/admin/shelves/${encodeURIComponent(slug)}`, b),
    deleteShelf: (slug: string) => request<void>("DELETE", `/v1/admin/shelves/${encodeURIComponent(slug)}`),
    orderShelves: (slugs: string[]) => request<AdminShelves>("PUT", "/v1/admin/shelves/order", { slugs }),
    hideFrontShelf: (key: string, hidden: boolean) =>
      request<AdminShelves>("POST", `/v1/admin/front-shelves/${encodeURIComponent(key)}`, { hidden }),
    legalDocuments: () => request<AdminLegalDocument[]>("GET", "/v1/admin/legal"),
    /** A sensitive action: may wait for a second person (202). */
    publishLegal: (kind: string, summary: string, body_md: string) =>
      request<unknown>("POST", `/v1/admin/legal/${encodeURIComponent(kind)}`, { summary, body_md }),
    withdrawLegal: (kind: string, version: string, reason: string) =>
      request<void>(
        "POST",
        `/v1/admin/legal/${encodeURIComponent(kind)}/${encodeURIComponent(version)}/withdraw`,
        { reason },
      ),
    featured: () => request<AdminFeatured[]>("GET", "/v1/admin/featured"),
    setFeatured: (apps: Array<{ slug: string; blurb: string }>) =>
      request<AdminFeatured[]>("PUT", "/v1/admin/featured", { apps }),
    apps: (status?: AppStatus, q?: string) =>
      request<AdminApp[]>("GET", `/v1/admin/apps${query({ status, q: q || undefined })}`),
    /** Everything about one app: listing, releases, money, reports, decisions, history. */
    appRecord: (slug: string) =>
      request<AdminAppRecord>("GET", `/v1/admin/apps/${encodeURIComponent(slug)}/record`),
    /** Staff fix a store page directly; the maker is told what changed and why. */
    editListing: (slug: string, body: ListingEdit) =>
      request<{ changed: string[] }>("PATCH", `/v1/admin/apps/${encodeURIComponent(slug)}/listing`, body),
    audit: (q?: string, before?: number) =>
      request<AuditEntry[]>("GET", `/v1/admin/audit${query({ q: q || undefined, before })}`),
    money: () => request<AdminMoney>("GET", "/v1/admin/money"),
    findUsers: (q: string) => request<AdminFoundUser[]>("GET", `/v1/admin/users${query({ q })}`),
    /** Looking is written to the audit log. */
    user: (id: string) => request<AdminUserDetails>("GET", `/v1/admin/users/${encodeURIComponent(id)}`),
    decideUser: (id: string, body: { action: "warn" | "suspend_account"; rule: string; facts: string; law?: string }) =>
      request<unknown>("POST", `/v1/admin/users/${encodeURIComponent(id)}/decide`, body),
    liftSuspension: (id: string, reason: string) =>
      request<unknown>("POST", `/v1/admin/users/${encodeURIComponent(id)}/lift-suspension`, { reason }),
    /** Threads AI agents wrote to the admins that you can see, newest activity first. */
    aiMessages: (category?: AiMessageCategory, status: "open" | "closed" = "open") =>
      request<AiThreadSummary[]>("GET", `/v1/admin/ai-messages${query({ category, status })}`),
    /** One thread with every message; reading it marks it read for you. */
    aiMessage: (id: string) => request<AiThread>("GET", `/v1/admin/ai-messages/${encodeURIComponent(id)}`),
    replyAiMessage: (id: string, body: string) =>
      request<{ id: string }>("POST", `/v1/admin/ai-messages/${encodeURIComponent(id)}/reply`, { body }),
    closeAiMessage: (id: string) => request<void>("POST", `/v1/admin/ai-messages/${encodeURIComponent(id)}/close`),
    reopenAiMessage: (id: string) => request<void>("POST", `/v1/admin/ai-messages/${encodeURIComponent(id)}/reopen`),
    fileAiMessage: (id: string, category: AiMessageCategory) =>
      request<void>("PUT", `/v1/admin/ai-messages/${encodeURIComponent(id)}/category`, { category }),
    /** Removes the whole thread (audit-logged without its text). */
    deleteAiMessage: (id: string) => request<void>("DELETE", `/v1/admin/ai-messages/${encodeURIComponent(id)}`),
    blockAiSender: (userId: string, reason: string) =>
      request<void>("POST", `/v1/admin/ai-senders/${encodeURIComponent(userId)}/block`, { reason }),
    /** Whether AI agents see this staff member's name (yours, or anyone's for an owner). */
    setListedByName: (userId: string, listed: boolean) =>
      request<void>("PUT", `/v1/admin/staff/${encodeURIComponent(userId)}/listed-by-name`, { listed }),
    unblockAiSender: (userId: string) =>
      request<void>("DELETE", `/v1/admin/ai-senders/${encodeURIComponent(userId)}/block`),
    approve: (id: string) =>
      request<void>("POST", `/v1/admin/releases/${id}/approve`),
    reject: (id: string, reason: string) =>
      request<void>("POST", `/v1/admin/releases/${id}/reject`, { reason }),
    /** Clears a scan finding or Defender result judged a false positive (logged). */
    closeAbuse: (id: string, note: string, acted: boolean) =>
      request<void>("POST", `/v1/admin/abuse/${id}/dismiss`, { note, acted }),
    /** Keeps a live release a re-scan flagged, as a false positive (logged). */
    clearRescan: (id: string, platform: string, reason: string) =>
      request<void>(
        "POST",
        `/v1/admin/releases/${id}/files/${encodeURIComponent(platform)}/rescan-clear`,
        { reason },
      ),
    clearScan: (id: string, platform: string, reason: string) =>
      request<void>(
        "POST",
        `/v1/admin/releases/${id}/files/${encodeURIComponent(platform)}/clear`,
        { reason },
      ),
    approveChange: (id: string) =>
      request<void>("POST", `/v1/admin/changes/${id}/approve`),
    rejectChange: (id: string, reason: string) =>
      request<void>("POST", `/v1/admin/changes/${id}/reject`, { reason }),
    approveMedia: (id: string) =>
      request<void>("POST", `/v1/admin/media/${id}/approve`),
    rejectMedia: (id: string, reason: string) =>
      request<void>("POST", `/v1/admin/media/${id}/reject`, { reason }),
    approvePortfolio: (id: string) =>
      request<void>("POST", `/v1/admin/portfolio/${id}/approve`),
    rejectPortfolio: (id: string, reason: string) =>
      request<void>("POST", `/v1/admin/portfolio/${id}/reject`, { reason }),
    jobRecord: (id: string) =>
      requestText("GET", `/v1/admin/jobs/${id}/record`),
    suggest: (problem: string, suggestion: string) =>
      request<void>("POST", `/v1/admin/problems/${problem}/suggest`, {
        suggestion,
      }),
    closeProblem: (problem: string, note: string) =>
      request<void>("POST", `/v1/admin/problems/${problem}/close`, { note }),
    decideDispute: (
      job: string,
      body: {
        outcome: "remove" | "note" | "keep";
        reason: string;
        note?: string;
      },
    ) => request<void>("POST", `/v1/admin/review-disputes/${job}/decide`, body),
    tokens: () => request<AdminToken[]>("GET", "/v1/admin/tokens"),
    createToken: (body: {
      name: string;
      scope: AdminToken["scope"];
      days: number;
    }) =>
      // `token` once; `mcp_url` is the admin MCP address (no command with the token in it).
      request<AdminToken & { token: string; mcp_url: string }>("POST", "/v1/admin/tokens", body),
    revokeToken: (id: string) =>
      request<void>("POST", `/v1/admin/tokens/${id}/revoke`),
    revoke: (id: string, reason: string) =>
      request<void>("POST", `/v1/admin/releases/${id}/revoke`, { reason }),
    list: (slug: string) =>
      request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/list`),
    donationHolds: (show: "open" | "all" = "open") =>
      request<AdminDonationHold[]>("GET", `/v1/admin/donation-holds?show=${show}`),
    /** "off" is a sensitive action: it may wait for a second approval. */
    holdDonations: (slug: string, body: { kind: "review" | "off"; reason: string; ground?: string }) =>
      request<{ hold?: string; pending_action?: string } | undefined>(
        "POST",
        `/v1/admin/apps/${encodeURIComponent(slug)}/donation-hold`,
        body,
      ),
    liftDonationHold: (id: string, note: string) =>
      request<void>("POST", `/v1/admin/donation-holds/${encodeURIComponent(id)}/lift`, { note }),
    answerDonationHoldRequest: (id: string, lift: boolean, answer: string) =>
      request<void>(
        "POST",
        `/v1/admin/donation-hold-requests/${encodeURIComponent(id)}/answer`,
        { lift, answer },
      ),
    delist: (slug: string, rule: string, reason: string) =>
      request<void>(
        "POST",
        `/v1/admin/apps/${encodeURIComponent(slug)}/delist`,
        { rule, reason },
      ),
    act: (
      id: string,
      body: {
        action: string;
        rule: string;
        law?: string;
        facts: string;
        outcome: string;
        external_reference?: string;
      },
    ) =>
      request<{ decision: string }>(
        "POST",
        `/v1/admin/reports/${id}/act`,
        body,
      ),
    dismiss: (id: string, outcome: string, external_reference?: string) =>
      request<void>("POST", `/v1/admin/reports/${id}/dismiss`, {
        outcome,
        external_reference,
      }),
    /** stays_out: reversed, but the app can't go back in the store yet (what the maker is told). */
    decideAppeal: (id: string, reverse: boolean, reason: string) =>
      request<{ stays_out: string | null }>(
        "POST",
        `/v1/admin/decisions/${id}/appeal`,
        { reverse, reason },
      ),
    stage: (slug: string, stage: AppStage) =>
      request<void>(
        "POST",
        `/v1/admin/apps/${encodeURIComponent(slug)}/stage`,
        { stage },
      ),
  },
  addToLibrary: (slug: string) =>
    request<void>("PUT", `/v1/me/library/${encodeURIComponent(slug)}`),
  removeFromLibrary: (slug: string) =>
    request<void>("DELETE", `/v1/me/library/${encodeURIComponent(slug)}`),
  /** Report (or take back) that the app doesn't work in sandbox mode. */
  sandboxReport: (slug: string, on: boolean) =>
    request<void>(on ? "PUT" : "DELETE", `/v1/apps/${encodeURIComponent(slug)}/sandbox-report`),
  desktopRequest: (id: string) =>
    request<{ device_name: string; expires_at: string }>(
      "GET",
      `/v1/auth/desktop/requests/${encodeURIComponent(id)}`,
    ),
  approveDesktop: (id: string) =>
    request<{ redirect_url: string }>(
      "POST",
      `/v1/auth/desktop/requests/${encodeURIComponent(id)}/approve`,
    ),

  providers: () =>
    request<Record<SignInProvider, boolean>>("GET", "/v1/auth/providers"),
  signingKeys: {
    list: () =>
      request<
        Array<{
          id: string;
          name: string;
          fingerprint: string;
          created_at: string;
          active_from: string;
          active: boolean;
        }>
      >("GET", "/v1/me/signing-keys"),
    add: (name: string, public_key: string) =>
      request<unknown>("POST", "/v1/me/signing-keys", { name, public_key }),
    remove: (id: string) =>
      request<void>("DELETE", `/v1/me/signing-keys/${encodeURIComponent(id)}`),
  },
  passkeys: {
    list: () =>
      request<
        Array<{
          id: string;
          name: string;
          created_at: string;
          last_used_at: string | null;
        }>
      >("GET", "/v1/me/passkeys"),
    remove: (id: string) =>
      request<void>("DELETE", `/v1/me/passkeys/${encodeURIComponent(id)}`),
    registerStart: () =>
      request<{
        challenge_id: string;
        options: { publicKey: Record<string, unknown> };
      }>("POST", "/v1/me/passkeys/register/start"),
    registerFinish: (challenge_id: string, name: string, credential: unknown) =>
      request<unknown>("POST", "/v1/me/passkeys/register/finish", {
        challenge_id,
        name,
        credential,
      }),
    signInStart: () =>
      request<{
        challenge_id: string;
        options: { publicKey: Record<string, unknown> };
      }>("POST", "/v1/auth/passkey/start"),
    signInFinish: (challenge_id: string, credential: unknown) =>
      request<unknown>("POST", "/v1/auth/passkey/finish", {
        challenge_id,
        credential,
      }),
  },
  /** The sign-ins linked to the account (crates/slop-api/src/auth/mod.rs). */
  identities: () => request<LinkedIdentity[]>("GET", "/v1/me/identities"),
  unlink: (provider: string, subject: string) =>
    request<void>(
      "DELETE",
      `/v1/me/identities/${encodeURIComponent(provider)}/${encodeURIComponent(subject)}`,
    ),
  /** The profile picture (crates/slop-api/src/avatars.rs). */
  picture: () => request<MyPicture>("GET", "/v1/me/avatar"),
  /** A linked sign-in's picture, or the letter (provider null). */
  setPicture: (provider: SignInProvider | null, subject?: string) =>
    request<MyPicture>("PUT", "/v1/me/avatar", { provider, subject }),
  /** Combining with the account of a sign-in being linked (auth/merge.rs). */
  combine: {
    preview: () => request<CombinePreview>("GET", "/v1/auth/combine"),
    confirm: () => request<void>("POST", "/v1/auth/combine"),
    cancel: () => request<void>("DELETE", "/v1/auth/combine"),
  },
  /** How to prove you own a repository, and whether you have (repo_owner.rs). */
  repoOwnership: (repo_url: string, repo_token?: RepoTokenInput) =>
    request<RepoOwnership>("POST", "/v1/repos/ownership", {
      repo_url,
      repo_token,
    }),
  checkRepoCode: (repo_url: string, repo_token?: RepoTokenInput) =>
    request<RepoOwnership>("POST", "/v1/repos/ownership/check", {
      repo_url,
      repo_token,
    }),
  /** A private repository outside GitHub: its read-only token (never shown again once saved). */
  repoToken: {
    status: (slug: string) =>
      request<RepoTokenStatus>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/repo-token`,
      ),
    save: (slug: string, t: RepoTokenInput) =>
      request<RepoTokenStatus>(
        "PUT",
        `/v1/apps/${encodeURIComponent(slug)}/repo-token`,
        t,
      ),
    forget: (slug: string) =>
      request<void>(
        "DELETE",
        `/v1/apps/${encodeURIComponent(slug)}/repo-token`,
      ),
  },
  /** The newest desktop app build for a platform (for the download button). */
  desktopLatest: (platform: "windows" | "mac" | "linux-deb" | "linux-rpm") =>
    request<{ version: string; url: string }>(
      "GET",
      `/v1/desktop/latest/${platform}`,
    ),
  /** What this server checks, so pages only claim what's true. */
  server: () =>
    request<{
      scanning: boolean;
      maker_signatures_required: boolean;
      simulation: boolean;
    }>("GET", "/v1/server"),
  ratings: (slug: string) =>
    request<Ratings>("GET", `/v1/apps/${encodeURIComponent(slug)}/ratings`),
  /** The maker's own breakdown by version and platform. */
  ratingsBreakdown: (slug: string) =>
    request<RatingBreakdown>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/ratings/breakdown`,
    ),
  featured: () => request<FeaturedApp[]>("GET", "/v1/featured"),
  vote: (slug: string, vote: "not_slop" | "slop" | null) =>
    request<Ratings>("POST", `/v1/apps/${encodeURIComponent(slug)}/vote`, {
      vote,
    }),
  forYou: () => request<ForYou>("GET", "/v1/me/for-you"),
  people: {
    list: (q: PeopleQuery = {}) =>
      request<{
        people: CreativeCard[];
        skills: Array<Skill & { count: number }>;
      }>("GET", `/v1/people${query(q)}`),
    skills: () => request<Skill[]>("GET", "/v1/people/skills"),
    page: (handle: string) =>
      request<CreativePage>("GET", `/v1/people/${encodeURIComponent(handle)}`),
    mine: () => request<MyProfile>("GET", "/v1/me/creative"),
    save: (p: ProfileInput) => request<MyProfile>("PUT", "/v1/me/creative", p),
    portfolio: () =>
      request<PortfolioOverview>("GET", "/v1/me/creative/portfolio"),
    checkPortfolio: (repo_url?: string) =>
      request<PortfolioOverview>(
        "POST",
        "/v1/me/creative/portfolio/check",
        repo_url ? { repo_url } : {},
      ),
    setPortfolioOrder: (keys: string[]) =>
      request<PortfolioOverview>("PUT", "/v1/me/creative/portfolio/order", {
        keys,
      }),
    getPaid: () =>
      request<{ url: string | null; status: string }>(
        "POST",
        "/v1/me/creative/stripe",
      ),
    refreshPaid: () =>
      request<{ url: string | null; status: string }>(
        "POST",
        "/v1/me/creative/stripe/refresh",
      ),
    reply: (jobId: string, reply: string) =>
      request<void>(
        "PUT",
        `/v1/people/reviews/${encodeURIComponent(jobId)}/reply`,
        { reply },
      ),
    hire: (
      handle: string,
      body: {
        app_slug: string;
        skill: string;
        service_id?: string;
        request_md: string;
        wanted_by?: string;
        client_is_business: boolean;
        inquiry: boolean;
        exact_wording_ack: boolean;
        confirm_adult: boolean;
      },
    ) =>
      request<Job>(
        "POST",
        `/v1/people/${encodeURIComponent(handle)}/hire`,
        body,
      ),
    jobs: () =>
      request<{
        as_client: JobCard[];
        as_creative: JobCard[];
        waiting_on_you: number;
      }>("GET", "/v1/me/jobs"),
    job: (id: string) =>
      request<Job>("GET", `/v1/jobs/${encodeURIComponent(id)}`),
    step: (id: string, step: string, body: unknown = {}) =>
      request<Job>(
        step === "review" || step === "review/dispute-answer" ? "PUT" : "POST",
        `/v1/jobs/${encodeURIComponent(id)}/${step}`,
        body,
      ),
    record: (id: string) =>
      requestText("GET", `/v1/jobs/${encodeURIComponent(id)}/record`),
    conversationCopy: (id: string) =>
      request<ConversationCopy>(
        "GET",
        `/v1/jobs/${encodeURIComponent(id)}/conversation`,
      ),
    submitCopy: (id: string, copy: ConversationCopy) =>
      request<{ verified_messages: number; unverified_messages: number }>(
        "POST",
        `/v1/jobs/${encodeURIComponent(id)}/evidence`,
        { copy },
      ),
    disputeReview: (jobId: string, reason: string, statement: string) =>
      request<void>(
        "POST",
        `/v1/people/reviews/${encodeURIComponent(jobId)}/dispute`,
        { reason, statement },
      ),
    pay: (id: string) =>
      request<{ url: string | null; job: Job }>(
        "POST",
        `/v1/jobs/${encodeURIComponent(id)}/pay`,
      ),
    checkPayment: (id: string) =>
      request<Job>("POST", `/v1/jobs/${encodeURIComponent(id)}/payment/check`),
  },
  simUsers: () => request<SimUser[]>("GET", "/v1/sim/users"),
  simLogin: (id: string) =>
    request<void>("POST", `/v1/sim/login/${encodeURIComponent(id)}`),
  /** Full-page navigation target that starts GitHub sign-in. */
  githubSignInUrl: (returnTo: string) =>
    `${API_BASE}/v1/auth/github/start${query({ return_to: returnTo })}`,
  signInUrl: (provider: SignInProvider, returnTo: string) =>
    `${API_BASE}/v1/auth/${provider}/start${query({ return_to: returnTo })}`,
  /** Linking another sign-in: signs in again first if needed, then to the provider. */
  linkUrl: (provider: SignInProvider, returnTo = "/account/security#sign-ins") =>
    `${API_BASE}/v1/auth/${provider}/link${query({ return_to: returnTo })}`,
  signupInfo: () => request<SignupInfo>("GET", "/v1/auth/signup"),
  signup: (body: {
    display_name: string;
    country_code: string;
    age_confirmed: boolean;
    accept_terms: boolean;
  }) =>
    request<{ user: Me; return_to: string }>("POST", "/v1/auth/signup", body),
  signout: () => request<void>("POST", "/v1/auth/signout"),
  me: () => request<Me>("GET", "/v1/me"),
  updateMe: (changes: {
    share_playtime?: boolean;
    personalize?: boolean;
    profile_public?: boolean;
  }) => request<void>("PATCH", "/v1/me", changes),
  players: (slug: string) =>
    request<{
      playing_now: number;
      peak_this_month: number;
      daily_peaks: Array<{ day: string; peak: number }>;
    }>("GET", `/v1/apps/${encodeURIComponent(slug)}/players`),
  reviews: {
    list: (slug: string, vote?: "not_slop" | "slop", offset?: number) =>
      request<{
        reviews: AppReview[];
        not_slop: number;
        slop: number;
        next_offset: number | null;
        can_reply: boolean;
      }>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/reviews${query({ vote, offset })}`,
      ),
    reply: (slug: string, id: string, body: string) =>
      request<void>("PUT", `/v1/apps/${encodeURIComponent(slug)}/reviews/${encodeURIComponent(id)}/reply`, { body }),
    removeReply: (slug: string, id: string) =>
      request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/reviews/${encodeURIComponent(id)}/reply`),
    write: (slug: string, body: string) =>
      request<void>("PUT", `/v1/apps/${encodeURIComponent(slug)}/review`, {
        body,
      }),
    remove: (slug: string) =>
      request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/review`),
  },
  private: {
    mine: () =>
      request<{
        mine: AppSummary[];
        shared: Array<AppSummary & { owner: string }>;
      }>("GET", "/v1/me/private"),
    shares: (slug: string) =>
      request<
        Array<{
          user_id: string | null;
          team: string | null;
          name: string;
          shared_at: string;
        }>
      >("GET", `/v1/apps/${encodeURIComponent(slug)}/shares`),
    share: (slug: string, to: { user?: string; team?: string }) =>
      request<
        Array<{
          user_id: string | null;
          team: string | null;
          name: string;
          shared_at: string;
        }>
      >("POST", `/v1/apps/${encodeURIComponent(slug)}/shares`, to),
    unshareTeam: (slug: string, team: string) =>
      request<void>(
        "DELETE",
        `/v1/apps/${encodeURIComponent(slug)}/team-shares/${encodeURIComponent(team)}`,
      ),
    unshare: (slug: string, userId: string) =>
      request<void>(
        "DELETE",
        `/v1/apps/${encodeURIComponent(slug)}/shares/${encodeURIComponent(userId)}`,
      ),
  },
  push: {
    key: () => request<{ public_key: string }>("GET", "/v1/push/key"),
    subscribe: (subscription: PushSubscriptionJSON) =>
      request<void>("POST", "/v1/me/push", subscription),
    unsubscribe: (endpoint: string) =>
      request<void>("DELETE", "/v1/me/push", { endpoint }),
  },
  friends: {
    list: () => request<Friends>("GET", "/v1/me/friends"),
    /** A profile link or user ID; "friends" when they had already asked you. */
    request: (user: string) =>
      request<{ status: "requested" | "friends" }>("POST", "/v1/me/friends", {
        user,
      }),
    accept: (id: string) =>
      request<void>("POST", `/v1/me/friends/${encodeURIComponent(id)}/accept`),
    /** Removes a friend, declines their request or cancels yours. */
    remove: (id: string) =>
      request<void>("DELETE", `/v1/me/friends/${encodeURIComponent(id)}`),
    blocks: () => request<FriendPerson[]>("GET", "/v1/me/blocks"),
    block: (id: string) =>
      request<void>("PUT", `/v1/me/blocks/${encodeURIComponent(id)}`),
    unblock: (id: string) =>
      request<void>("DELETE", `/v1/me/blocks/${encodeURIComponent(id)}`),
    connection: (id: string) =>
      request<Connection>(
        "GET",
        `/v1/users/${encodeURIComponent(id)}/connection`,
      ),
    chatSettings: () =>
      request<{ from_anyone: boolean; locked: boolean }>(
        "GET",
        "/v1/me/chat-settings",
      ),
    setChatSettings: (from_anyone: boolean) =>
      request<{ from_anyone: boolean; locked: boolean }>(
        "PUT",
        "/v1/me/chat-settings",
        { from_anyone },
      ),
  },
  teams: {
    list: () => request<TeamCard[]>("GET", "/v1/teams"),
    mine: () =>
      request<{ teams: TeamCard[]; invites: TeamCard[]; team_chats: boolean }>(
        "GET",
        "/v1/me/teams",
      ),
    create: (body: {
      slug: string;
      name: string;
      kind: "player" | "official";
      about: string;
      join_policy?: string;
      github_org?: string;
    }) => request<{ slug: string }>("POST", "/v1/teams", body),
    page: (slug: string) =>
      request<TeamPage>("GET", `/v1/teams/${encodeURIComponent(slug)}`),
    update: (
      slug: string,
      body: {
        name?: string;
        about?: string;
        join_policy?: string;
        dns_domain?: string;
      },
    ) => request<void>("PATCH", `/v1/teams/${encodeURIComponent(slug)}`, body),
    join: (slug: string) =>
      request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/join`),
    leave: (slug: string) =>
      request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/leave`),
    invite: (slug: string, user: string) =>
      request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/invites`, {
        user,
      }),
    accept: (slug: string) =>
      request<void>(
        "POST",
        `/v1/teams/${encodeURIComponent(slug)}/invites/accept`,
      ),
    decline: (slug: string) =>
      request<void>(
        "POST",
        `/v1/teams/${encodeURIComponent(slug)}/invites/decline`,
      ),
    removeMember: (slug: string, userId: string) =>
      request<void>(
        "DELETE",
        `/v1/teams/${encodeURIComponent(slug)}/members/${encodeURIComponent(userId)}`,
      ),
    setRole: (slug: string, userId: string, role: string) =>
      request<void>(
        "PUT",
        `/v1/teams/${encodeURIComponent(slug)}/members/${encodeURIComponent(userId)}`,
        { role },
      ),
    pick: (slug: string, app_slug: string, note: string) =>
      request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/picks`, {
        app_slug,
        note,
      }),
    unpick: (slug: string, id: string) =>
      request<void>(
        "DELETE",
        `/v1/teams/${encodeURIComponent(slug)}/picks/${encodeURIComponent(id)}`,
      ),
    addEvent: (
      slug: string,
      body: { title: string; starts_at: string; app_slug?: string },
    ) =>
      request<void>(
        "POST",
        `/v1/teams/${encodeURIComponent(slug)}/events`,
        body,
      ),
    removeEvent: (slug: string, id: string) =>
      request<void>(
        "DELETE",
        `/v1/teams/${encodeURIComponent(slug)}/events/${encodeURIComponent(id)}`,
      ),
    verify: (slug: string) =>
      request<{ verified: boolean; checks: TeamPage["checks"] }>(
        "POST",
        `/v1/teams/${encodeURIComponent(slug)}/verify`,
      ),
  },
  registerBranch: (
    parent: string,
    body: {
      slug: string;
      title: string;
      summary: string;
      repo_url: string;
      license_spdx: string;
      kind: string;
      note: string;
      platforms: string[];
      description_md?: string;
      /** At least the original's; the original's when left out. */
      min_age?: number;
      content_labels?: ContentLabel[];
      online?: OnlineKind[];
      online_hosts?: string[];
      sandbox_ready?: boolean;
    },
  ) =>
    request<{ slug: string }>(
      "POST",
      `/v1/apps/${encodeURIComponent(parent)}/branches`,
      body,
    ),
  profile: (id: string) =>
    request<UserProfile>("GET", `/v1/users/${encodeURIComponent(id)}`),
  donations: {
    info: (slug: string) =>
      request<DonateInfo>("GET", `/v1/apps/${encodeURIComponent(slug)}/donate`),
    quote: (slug: string, amount_minor: number) =>
      request<DonationQuote & { currency: string; consent: string | null }>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/donate/quote${query({ amount_minor })}`,
      ),
    start: (
      slug: string,
      body: {
        amount_minor: number;
        monthly: boolean;
        share_name: boolean;
        message?: string;
        consent: boolean;
        confirm_adult: boolean;
        challenge?: { id: string; nonce: string };
        /** The original's share of a donation to this branch, and what the branch gets next. */
        share_for?: string;
        rest_minor?: number;
        /** A donation to a branch: the paid share to the original it follows. */
        share_donation?: string;
      },
    ) =>
      request<{ url: string | null; id: string; done: boolean }>(
        "POST",
        `/v1/apps/${encodeURIComponent(slug)}/donate`,
        body,
      ),
    check: (id: string) =>
      request<void>("POST", `/v1/me/donations/${encodeURIComponent(id)}/check`),
    mine: () =>
      request<{
        donations: MyDonation[];
        server_donations: MyServerDonation[];
      }>("GET", "/v1/me/donations"),
    cancel: (id: string) =>
      request<void>(
        "POST",
        `/v1/me/server-donations/${encodeURIComponent(id)}/cancel`,
      ),
    settings: (slug: string) =>
      request<DonationSettings>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/donations/settings`,
      ),
    saveSettings: (
      slug: string,
      body: {
        donations_on: boolean;
        donation_suggested_minor: number;
        donation_currency: string;
        runs_servers: boolean;
        servers_statement: string | null;
      },
    ) =>
      request<DonationSettings>(
        "PUT",
        `/v1/apps/${encodeURIComponent(slug)}/donations/settings`,
        body,
      ),
    /** A hold staff placed on this app's donations, for its maker. */
    hold: (slug: string) =>
      request<{ open: DonationHold | null; can_ask_at: string | null }>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/donations/hold`,
      ),
    askToLift: (slug: string, message: string) =>
      request<void>(
        "POST",
        `/v1/apps/${encodeURIComponent(slug)}/donations/hold/request`,
        { message },
      ),
    serversShutDown: (slug: string) =>
      request<{ cancelled: number }>(
        "POST",
        `/v1/apps/${encodeURIComponent(slug)}/servers/shut-down`,
      ),
    summary: (slug: string) =>
      request<DonationSummary>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/donations`,
      ),
    refund: (id: string) =>
      request<void>("POST", `/v1/donations/${encodeURIComponent(id)}/refund`),
    account: () => request<DonationAccount>("GET", "/v1/me/donation-account"),
    /** Needs a sign-in from the last few minutes; takes effect after 72 hours. */
    changeAccount: (kind: "switch" | "disconnect", return_to?: string) =>
      request<{ url: string | null; status: string }>(
        "POST",
        "/v1/me/donation-account/change",
        { kind, return_to },
      ),
    cancelAccountChange: () =>
      request<void>("POST", "/v1/me/donation-account/change/cancel"),
    challenge: () => request<PowChallenge>("POST", "/v1/donations/challenge"),
    setVat: (vat_number: string | null) =>
      request<{
        vat_number: string | null;
        checked: boolean;
        note: string | null;
      }>("PUT", "/v1/me/vat", { vat_number }),
    onboard: (body: {
      tax_notice_ack: boolean;
      confirm_adult: boolean;
      return_to?: string;
    }) =>
      request<{ url: string | null; status: string }>(
        "POST",
        "/v1/me/donation-account",
        body,
      ),
    refreshAccount: () =>
      request<{ url: string | null; status: string }>(
        "POST",
        "/v1/me/donation-account/refresh",
      ),
    feeStatements: () =>
      request<{ statements: FeeStatement[]; vat: VatTreatment }>(
        "GET",
        "/v1/me/fee-statements",
      ),
  },
  interested: {
    mine: () =>
      request<Array<AppSummary & { interested_at: string }>>(
        "GET",
        "/v1/me/interested",
      ),
    add: (slug: string) =>
      request<void>("PUT", `/v1/me/interested/${encodeURIComponent(slug)}`),
    remove: (slug: string) =>
      request<void>("DELETE", `/v1/me/interested/${encodeURIComponent(slug)}`),
    /** Upcoming apps for this viewer, each with why; the most anticipated when not personal. */
    forYou: () =>
      request<Array<AppSummary & { reason: string }>>(
        "GET",
        "/v1/upcoming/for-you",
      ),
  },
  notifications: {
    list: () =>
      request<{ unread: number; items: Notification[] }>(
        "GET",
        "/v1/me/notifications",
      ),
    read: (ids: string[] = []) =>
      request<void>("POST", "/v1/me/notifications/read", { ids }),
    prefs: () => request<EmailPrefs>("GET", "/v1/me/email-prefs"),
    setPrefs: (changes: Partial<Record<Notification["category"], boolean>>) =>
      request<EmailPrefs>("PUT", "/v1/me/email-prefs", changes),
  },
  legal: {
    index: () => request<LegalIndexEntry[]>("GET", "/v1/legal"),
    document: (kind: string, version?: string) =>
      request<LegalDocument>(
        "GET",
        `/v1/legal/${encodeURIComponent(kind)}${query({ version })}`,
      ),
    notices: () => request<LegalNotice[]>("GET", "/v1/me/legal"),
    mine: (kind: LegalKind) =>
      request<MyLegalDocument>("GET", `/v1/me/legal/${encodeURIComponent(kind)}`),
    accept: (kind: LegalKind, version: string) =>
      request<void>("POST", "/v1/me/legal/accept", { kind, version }),
  },
  reports: {
    reasons: (kind: ReportKind) =>
      request<ReportReason[]>("GET", `/v1/reports/reasons${query({ kind })}`),
    send: (body: ReportBody) =>
      request<{ reference: string; due_at: string }>(
        "POST",
        "/v1/reports",
        body,
      ),
    mine: () => request<MyReport[]>("GET", "/v1/me/reports"),
  },
  decisions: {
    mine: () => request<ModerationDecision[]>("GET", "/v1/me/decisions"),
    appeal: (id: string, appeal: string, counter_notice?: CounterNotice) =>
      request<ModerationDecision>(
        "POST",
        `/v1/me/decisions/${encodeURIComponent(id)}/appeal`,
        { appeal, counter_notice },
      ),
  },
  /** Everything linked to the account, as JSON text to save. */
  exportData: () => requestText("GET", "/v1/me/export"),
  agentTokens: () => request<AgentToken[]>("GET", "/v1/me/maker-tokens"),
  createAgentToken: (body: { name: string; scope: AgentToken["scope"]; apps: string[]; people: boolean; days: number }) =>
    request<AgentToken & { secret: string; mcp_url: string }>(
      "POST",
      "/v1/me/maker-tokens",
      body,
    ),
  revokeAgentToken: (id: string) => request<void>("DELETE", `/v1/me/maker-tokens/${encodeURIComponent(id)}`),
  /** Fixes for the problems in an app's newest release. */
  fixes: (slug: string) =>
    request<{ release: { id: string; version: string; status: string } | null; fixes: Fix[]; next: string }>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/fixes`,
    ),
  offers: (slug: string) => request<UpstreamOffer[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/offers`),
  offerFix: (slug: string, pull_request_url: string, message: string) =>
    request<UpstreamOffer>("POST", `/v1/apps/${encodeURIComponent(slug)}/offers`, { pull_request_url, message }),
  answerOffer: (slug: string, id: string, decision: "accepted" | "declined", message: string) =>
    request<void>("POST", `/v1/apps/${encodeURIComponent(slug)}/offers/${encodeURIComponent(id)}/answer`, {
      decision,
      message: message.trim() || undefined,
    }),
  withdrawOffer: (slug: string, id: string) =>
    request<void>("POST", `/v1/apps/${encodeURIComponent(slug)}/offers/${encodeURIComponent(id)}/withdraw`),
  setReleaseNotes: (slug: string, id: string, notes: string) =>
    request<Release>("PATCH", `/v1/apps/${encodeURIComponent(slug)}/releases/${encodeURIComponent(id)}`, { notes }),
  closeAccount: (confirm: string) =>
    request<void>("POST", "/v1/me/close", { confirm }),
  sessions: () => request<SessionInfo[]>("GET", "/v1/me/sessions"),
  revokeSession: (id: string) =>
    request<void>("DELETE", `/v1/me/sessions/${encodeURIComponent(id)}`),
};

/** A request from police, a court or an authority (crates/slop-api/src/legal_requests.rs). */
export interface LegalRequest {
  id: string;
  reference: string;
  received_at: string;
  country: string;
  authority: string;
  kind: LegalRequestKind;
  legal_basis: string;
  asked: string;
  subject_user_id: string | null;
  subject_name: string | null;
  deadline: string | null;
  status: "open" | "answered" | "partly_answered" | "refused" | "withdrawn";
  response: string | null;
  user_notice: "pending" | "notified" | "barred" | "not_needed";
  created_by: string;
  answered_by: string | null;
  answered_at: string | null;
}

export type LegalRequestKind =
  | "subpoena"
  | "court_order"
  | "warrant"
  | "eu_production_order"
  | "preservation"
  | "emergency"
  | "uk_daa"
  | "removal_order"
  | "other";

export interface NewLegalRequest {
  received_at: string;
  country: string;
  authority: string;
  kind: LegalRequestKind;
  legal_basis: string;
  asked: string;
  subject_user_id?: string | null;
  deadline?: string | null;
}

export interface TransparencyReport {
  year: number;
  /** False while the year is still going. */
  complete: boolean;
  legal_requests: Record<"by_country" | "by_kind" | "by_outcome", Array<{ key: string; count: number }>>;
  reports: {
    by_reason: Array<{ key: string; count: number }>;
    by_outcome: Array<{ key: string; count: number }>;
    median_hours_to_decide: number | null;
  };
  decisions: { by_action: Array<{ key: string; count: number }>; automated: number };
  appeals: { by_outcome: Array<{ key: string; count: number }> };
}
