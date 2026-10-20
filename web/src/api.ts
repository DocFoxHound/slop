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

export type ContentLabel = "romance" | "suggestive" | "violence" | "horror" | "strong_language" | "gambling_themes";

/** A slide in the front page's Featured carousel. */
export type FeaturedApp = AppSummary & { blurb: string };

/** The maker's rating breakdown (crates/slop-api/src/votes.rs, METRICS §3, §5). */
export interface RatingGroup extends Tally {
  key: string | null;
}
export interface RatingBreakdown {
  by_version: RatingGroup[];
  by_platform: RatingGroup[];
  impact: Array<{ version: string; to_not_slop: number; to_slop: number; new_votes: number }>;
}

/** Updates for an installed app (crates/slop-api/src/library.rs). */
export interface AppUpdate {
  slug: string;
  latest: { release_id: string; version: string } | null;
  /** A newer release still being checked: "fetching" (downloading and scanning) or "awaiting_review". */
  pending: { version: string; status: "fetching" | "awaiting_review" } | null;
  /** The official app and its community builds, when there's more than one. */
  builds: Array<{ slug: string; title: string; official: boolean; platforms: string[] }>;
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

export interface DonationSettings {
  donations_on: boolean;
  donation_suggested_minor: number;
  donation_currency: string;
  runs_servers: boolean;
  servers_statement: string | null;
  servers_shut_down_at: string | null;
  account_status: "none" | "pending" | "active" | "restricted";
  active_server_donations: number;
  payments: "stripe" | "simulated" | "off";
}

export interface DonationSummary {
  currency_totals: Array<{ currency: string; donations: number; donated_minor: number; slop_fee_minor: number; refunded_minor: number }>;
  months: Array<{ month: string; currency: string; donations: number; donated_minor: number }>;
  active_server_donations: number | null;
  shared: Array<{ name: string; message: string | null; currency: string; donation_minor: number; paid_at: string }>;
  recent: Array<{ id: string; kind: string; currency: string; total_minor: number; status: string; refundable: boolean; paid_at: string }>;
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
  change: { kind: "switch" | "disconnect"; requested_at: string; effective_at: string; new_account_ready: boolean | null } | null;
}

export type SignInProvider = "github" | "google" | "gitlab" | "bitbucket" | "microsoft" | "discord";

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
}

export interface UserProfile {
  id: string;
  name: string;
  joined: string;
  public: boolean;
  is_me: boolean;
  creative_handle: string | null;
  apps: AppSummary[];
  votes: Array<AppSummary & { my_vote: "not_slop" | "slop"; review: string | null; voted_at: string }>;
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
  members: Array<{ user_id: string; name: string; role: "owner" | "admin" | "member"; joined_at: string }>;
  member_count: number;
  apps: AppSummary[];
  picks: Array<AppSummary & { id: string; note: string; by: string }>;
  events: Array<{ id: string; title: string; starts_at: string; app_slug: string | null; app_title: string | null; by: string; mine: boolean }>;
  shared: AppSummary[];
  hidden: boolean;
}

/** A notification in the bell (crates/slop-api/src/notify.rs). */
export interface Notification {
  id: string;
  category: "jobs" | "interested" | "moderation" | "donations" | "shared" | "teams" | "apps" | "chat" | "friends";
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
  categories: Array<{ category: Notification["category"]; label: string; email: boolean }>;
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
  | { kind: "video"; youtube_id: string; poster: RepoPicture | null; caption: string | null };

/** The order key the server uses for a gallery item. */
export function mediaKey(item: MediaItem): string {
  return item.kind === "image" ? `image:${item.path}` : `video:${item.youtube_id}`;
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
  scope: "read" | "review";
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
  community: Array<AppSummary & { author: string; branch_kind: string; branch_note: string | null; platforms: string[]; adds: string[] }>;
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
}

/** Slop's legal documents (crates/slop-api/src/legal.rs). */
export type LegalKind = "terms" | "privacy" | "content" | "publisher" | "people";

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
  versions: LegalVersion[];
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
export type ReportKind = "app" | "review" | "profile" | "message" | "app_review" | "team" | "chat_message";

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
  action: "delist_app" | "hide_review" | "hide_profile" | "suspend_account" | "warn";
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
  daily: Array<{ day: string; not_slop: number; slop: number; changed: number; withdrawn: number }>;
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
  since_new_work: (Partial<Tally> & { since: string | null; version: string | null; credits: string[] }) | null;
}

// ---- People (crates/slop-api/src/people) ------------------------------------------------------

export interface Skill {
  slug: string;
  name: string;
}

export type AiUse = "none" | "assisted" | "heavy";
export type Accent = "slime" | "ocean" | "grape" | "rose" | "ember" | "sun" | "forest" | "slate";
export type Section = "portfolio" | "services" | "reviews" | "worked_on" | "about";
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
  status: "live" | "waiting" | "hidden";
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
}

export type PortfolioWork =
  | { kind: "image"; path: string; url: string; width: number; height: number; title: string | null; caption: string | null }
  | { kind: "video"; youtube_id: string; poster: RepoPicture | null; title: string | null; caption: string | null }
  | { kind: "audio"; audio: { path: string; url: string; format: string }; poster: RepoPicture | null; title: string | null; caption: string | null };

export function workKey(w: PortfolioWork): string {
  return w.kind === "image" ? `image:${w.path}` : w.kind === "video" ? `video:${w.youtube_id}` : `audio:${w.audio.path}`;
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
  proposals: Array<{ id: string; mine: boolean; kind: "refund" | "cancel" | "redo" | "other"; amount_minor: number | null; note: string; status: string }>;
}

/** A side's own copy of a job conversation, as downloaded. */
export interface ConversationCopy {
  job_id: string;
  messages: Array<{ id: string; author_id: string; author: string; created_at: string; body: string; sha256: string }>;
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
  agreement: { text: string; sha256: string | null; template: string; accepted_at: string | null } | null;
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
    messages: Array<{ id: string; author: string; mine: boolean; body: string | null; sha256: string; created_at: string }>;
  };
  problem: JobProblem | null;
  /** For the creative: how this client has paid before. */
  client_record: { completed_jobs: number; paid_in_full: number; owing: number } | null;
  events: Array<{ kind: string; actor: string | null; note: string | null; links: string[]; created_at: string }>;
  actions: JobAction[];
  payment_mode: "stripe" | "simulated" | "off";
  review: {
    vote: "not_slop" | "slop";
    body: string;
    reply: string | null;
    hidden: boolean;
    mediator_note: string | null;
    dispute: { reason: string; statement: string; client_statement: string | null; status: string; decision_reason: string | null } | null;
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
  repo_url: string;
  /** Players have the app, so moving it is reviewed by a person. */
  repo_locked: boolean;
  auto_approval_ready: boolean;
  auto_approval_note: string | null;
  changes: AppChange[];
  /** Slop can't read the repository: no new releases until it can (SECURITY §12.2). */
  frozen: { reason: string; since: string; until: string } | null;
  /** Private Slop waiting for a reviewer to make it public. */
  public_requested: boolean;
  /** Community builds of Pulse games: null if not needed, else whether the handshake arrived. */
  pulse_required: boolean | null;
}

export type ChangeRequest =
  | { kind: "release"; release_id: string | null; note?: string }
  | { kind: "repo"; repo_url: string; note?: string };

export interface AdminFeatured {
  slug: string;
  title: string;
  blurb: string;
  /** In the store for everyone; a slide that isn't simply doesn't show. */
  listed: boolean;
}

export interface AdminQueue {
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
  changes: Array<AppChange & { app: AdminAppRef; players_get: string | null; notes: ReviewNote[] }>;
  media: Array<MediaSet & { app: AdminAppRef; live: MediaSet | null; notes: ReviewNote[] }>;
  portfolios: Array<{ handle: string; name: string; ai_use: AiUse; set: PortfolioSet; live: PortfolioSet | null; notes: ReviewNote[] }>;
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
    proposals: Array<{ kind: string; amount_minor: number | null; note: string; status: string }>;
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
    AdminAppRef & { approved_releases: number; pictures_pending: boolean; going_public: boolean; notes: ReviewNote[] }
  >;
  checks: { scanning: boolean; maker_signatures_required: boolean; signing_key: boolean; github_app: boolean; github_token: boolean };
}

export interface AdminApp extends AdminAppRef {
  releases: Array<{ id: string; version: string; reviewed_at: string | null }>;
  /** Releases still being checked or waiting for review. */
  pending_releases: number;
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
  kind: "web" | "desktop";
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  current: boolean;
}

export type Platform = "windows" | "mac" | "linux" | "android" | "linux-deb" | "linux-rpm";
/** What an app's store page says it runs on: the builds every release must include. */
export type AppPlatform = "windows" | "linux" | "linux-deb" | "linux-rpm";
export type FileKind = "archive" | "installer" | "apk";
export type ReleaseStatus = "fetching" | "awaiting_review" | "approved" | "rejected" | "failed" | "revoked";

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
  scan_result: "passed" | "note" | "on_hold" | "rejected" | "blocked" | "skipped" | null;
  scan_detail: string | null;
  /** Slop's scanner's report: engines, findings, notable capabilities. */
  scan_report: ScanReport | null;
  /** Microsoft Defender's check (Windows builds), from Slop Build or Slop Check. */
  defender: DefenderCheck | null;
  /** A reviewer cleared a finding as a false positive, and why. */
  cleared_reason: string | null;
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
  vulnerabilities: Array<{ package: string; version: string; ecosystem: string; id: string; summary: string }>;
  tools: Record<string, string>;
  checked: boolean;
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
  files: ReleaseFile[];
  /** Slop Build's source checks. */
  source_checks: SourceChecks | null;
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
  assets: Array<{ name: string; url: string; size: number; platform: Platform | null; kind: FileKind | null }>;
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
  files: Array<{ platform: Platform; kind: FileKind; url: string; exec?: string; args?: string[] }>;
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

export type AppChanges = Partial<Omit<NewApp, "slug" | "kind" | "stage" | "visibility" | "repo_token">>;

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
    | "fresh_slop";
  made_with_people?: boolean;
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
    if (value !== undefined && value !== null && value !== "") search.set(key, String(value));
  }
  const s = search.toString();
  return s ? `?${s}` : "";
}

/** In the desktop app, requests go through Rust, which holds the user's tokens. */
async function desktopRequest(method: string, path: string, body?: unknown): Promise<{ status: number; body: unknown }> {
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return await invoke<{ status: number; body: unknown }>("api_request", { methodName: method, path, body: body ?? null });
  } catch (err) {
    const e = err as { code?: string; message?: string };
    throw new ApiError(0, e.code ?? "network", e.message ?? "Can't reach Slop. Check your connection and try again.");
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  if (isDesktop) {
    const res = await desktopRequest(method, path, body);
    if (res.status >= 200 && res.status < 300) return res.body as T;
    const err = (res.body as { error?: { code?: string; message?: string; field?: string } } | null)?.error;
    throw new ApiError(res.status, err?.code ?? "http", err?.message ?? `Request failed (${res.status})`, err?.field);
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
    throw new ApiError(0, "network", "Can't reach Slop. Check your connection and try again.");
  }
  if (res.status === 204) return undefined as T;

  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const err = (data as { error?: { code?: string; message?: string; field?: string } } | null)?.error;
    throw new ApiError(res.status, err?.code ?? "http", err?.message ?? `Request failed (${res.status})`, err?.field);
  }
  return data as T;
}

/** A text response (job records). The desktop app returns JSON bodies only, so it gets text back as a string too. */
async function requestText(method: string, path: string): Promise<string> {
  if (isDesktop) {
    const res = await desktopRequest(method, path);
    if (res.status >= 200 && res.status < 300) return typeof res.body === "string" ? res.body : JSON.stringify(res.body);
    throw new ApiError(res.status, "http", `Request failed (${res.status})`);
  }
  const res = await fetch(API_BASE + path, { method, credentials: "same-origin" }).catch(() => {
    throw new ApiError(0, "network", "Can't reach Slop. Check your connection and try again.");
  });
  if (!res.ok) throw new ApiError(res.status, "http", `Request failed (${res.status})`);
  return res.text();
}

export const api = {
  categories: () => request<Category[]>("GET", "/v1/categories"),
  apps: (q: AppQuery = {}) => request<AppPage>("GET", `/v1/apps${query(q)}`),
  app: (slug: string) => request<AppDetail>("GET", `/v1/apps/${encodeURIComponent(slug)}`),
  createApp: (app: NewApp) => request<AppDetail>("POST", "/v1/apps", app),
  updateApp: (slug: string, changes: AppChanges) =>
    request<AppDetail>("PATCH", `/v1/apps/${encodeURIComponent(slug)}`, changes),
  myApps: () => request<AppSummary[]>("GET", "/v1/me/apps"),
  similar: (slug: string) => request<SimilarApps>("GET", `/v1/apps/${encodeURIComponent(slug)}/similar`),
  /** Pictures from the repository's .slop/media.json (crates/slop-api/src/media.rs). */
  media: (slug: string) => request<MediaOverview>("GET", `/v1/apps/${encodeURIComponent(slug)}/media`),
  checkMedia: (slug: string) => request<MediaOverview>("POST", `/v1/apps/${encodeURIComponent(slug)}/media/check`),
  setMediaOrder: (slug: string, keys: string[]) =>
    request<MediaOverview>("PUT", `/v1/apps/${encodeURIComponent(slug)}/media/order`, { keys }),
  /** The licence GitHub finds in a repository, for the Publish form. */
  detectLicense: (repoUrl: string) =>
    request<{ spdx: string | null; name: string | null; note: string | null }>(
      "GET",
      `/v1/repos/license${query({ url: repoUrl })}`,
    ),
  releases: (slug: string) => request<Release[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/releases`),
  githubReleases: (slug: string) => request<GithubRelease[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/github-releases`),
  submitGithubRelease: (slug: string, tag: string, files: NewRelease["files"]) =>
    request<Release>("POST", `/v1/apps/${encodeURIComponent(slug)}/releases/from-github`, { tag, files }),
  updateSettings: (slug: string) => request<UpdateSettings>("GET", `/v1/apps/${encodeURIComponent(slug)}/update-settings`),
  changeUpdateSettings: (slug: string, change: Partial<Pick<UpdateSettings, "watch_releases" | "auto_update">>) =>
    request<UpdateSettings>("PUT", `/v1/apps/${encodeURIComponent(slug)}/update-settings`, change),
  submitRelease: (slug: string, release: NewRelease) =>
    request<Release>("POST", `/v1/apps/${encodeURIComponent(slug)}/releases`, release),
  /** Whether an approved, signed build exists (the manifest itself is checked in Rust). */
  /** For installed apps: the release players get, a newer one being checked, and other builds. */
  libraryUpdates: (slugs: string[]) =>
    request<AppUpdate[]>("GET", `/v1/library/updates?apps=${encodeURIComponent(slugs.join(","))}`),
  hasBuild: (slug: string) =>
    request<unknown>("GET", `/v1/apps/${encodeURIComponent(slug)}/manifest`).then(
      () => true,
      (err: unknown) => {
        if (err instanceof ApiError && err.status === 404) return false;
        throw err;
      },
    ),
  slopBuildSetup: (slug: string) =>
    request<{
      supported: boolean;
      project_type: "rust" | "go" | "custom";
      detected: string | null;
      note: string | null;
      file_path: string;
      workflow: string;
      create_url: string | null;
      check_file_path: string;
      check_workflow: string;
      check_create_url: string | null;
    }>("GET", `/v1/apps/${encodeURIComponent(slug)}/slop-build`),
  manage: (slug: string) => request<ManageOverview>("GET", `/v1/apps/${encodeURIComponent(slug)}/manage`),
  /** "Check now" for a frozen app's repository; the freeze if it's still frozen. */
  repoCheck: (slug: string) =>
    request<ManageOverview["frozen"]>("POST", `/v1/apps/${encodeURIComponent(slug)}/repo-check`),
  makePublic: (slug: string) => request<void>("POST", `/v1/apps/${encodeURIComponent(slug)}/make-public`),
  cancelMakePublic: (slug: string) => request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/make-public`),
  requestChange: (slug: string, change: ChangeRequest) =>
    request<AppChange>("POST", `/v1/apps/${encodeURIComponent(slug)}/changes`, change),
  cancelChange: (slug: string, id: string) =>
    request<AppChange>("POST", `/v1/apps/${encodeURIComponent(slug)}/changes/${encodeURIComponent(id)}/cancel`),
  library: () => request<LibraryEntry[]>("GET", "/v1/me/library"),
  admin: {
    queue: () => request<AdminQueue>("GET", "/v1/admin/queue"),
    /** What needs an admin right now (ops.rs): stale backups, failed deploys, overdue reports… */
    ops: () => request<Array<{ key: string; title: string; detail: string; link: string; first_seen: string }>>("GET", "/v1/admin/ops"),
    featured: () => request<AdminFeatured[]>("GET", "/v1/admin/featured"),
    setFeatured: (apps: Array<{ slug: string; blurb: string }>) =>
      request<AdminFeatured[]>("PUT", "/v1/admin/featured", { apps }),
    apps: (status?: AppStatus) => request<AdminApp[]>("GET", `/v1/admin/apps${query({ status })}`),
    audit: () => request<AuditEntry[]>("GET", "/v1/admin/audit"),
    approve: (id: string) => request<void>("POST", `/v1/admin/releases/${id}/approve`),
    reject: (id: string, reason: string) => request<void>("POST", `/v1/admin/releases/${id}/reject`, { reason }),
    /** Clears a scan finding or Defender result judged a false positive (logged). */
    /** Keeps a live release a re-scan flagged, as a false positive (logged). */
    clearRescan: (id: string, platform: string, reason: string) =>
      request<void>("POST", `/v1/admin/releases/${id}/files/${encodeURIComponent(platform)}/rescan-clear`, { reason }),
    clearScan: (id: string, platform: string, reason: string) =>
      request<void>("POST", `/v1/admin/releases/${id}/files/${encodeURIComponent(platform)}/clear`, { reason }),
    approveChange: (id: string) => request<void>("POST", `/v1/admin/changes/${id}/approve`),
    rejectChange: (id: string, reason: string) => request<void>("POST", `/v1/admin/changes/${id}/reject`, { reason }),
    approveMedia: (id: string) => request<void>("POST", `/v1/admin/media/${id}/approve`),
    rejectMedia: (id: string, reason: string) => request<void>("POST", `/v1/admin/media/${id}/reject`, { reason }),
    approvePortfolio: (id: string) => request<void>("POST", `/v1/admin/portfolio/${id}/approve`),
    rejectPortfolio: (id: string, reason: string) => request<void>("POST", `/v1/admin/portfolio/${id}/reject`, { reason }),
    jobRecord: (id: string) => requestText("GET", `/v1/admin/jobs/${id}/record`),
    suggest: (problem: string, suggestion: string) => request<void>("POST", `/v1/admin/problems/${problem}/suggest`, { suggestion }),
    closeProblem: (problem: string, note: string) => request<void>("POST", `/v1/admin/problems/${problem}/close`, { note }),
    decideDispute: (job: string, body: { outcome: "remove" | "note" | "keep"; reason: string; note?: string }) =>
      request<void>("POST", `/v1/admin/review-disputes/${job}/decide`, body),
    tokens: () => request<AdminToken[]>("GET", "/v1/admin/tokens"),
    createToken: (body: { name: string; scope: AdminToken["scope"]; days: number }) =>
      request<AdminToken & { token: string }>("POST", "/v1/admin/tokens", body),
    revokeToken: (id: string) => request<void>("POST", `/v1/admin/tokens/${id}/revoke`),
    revoke: (id: string, reason: string) => request<void>("POST", `/v1/admin/releases/${id}/revoke`, { reason }),
    list: (slug: string) => request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/list`),
    delist: (slug: string, rule: string, reason: string) =>
      request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/delist`, { rule, reason }),
    act: (
      id: string,
      body: { action: string; rule: string; law?: string; facts: string; outcome: string; external_reference?: string },
    ) => request<{ decision: string }>("POST", `/v1/admin/reports/${id}/act`, body),
    dismiss: (id: string, outcome: string, external_reference?: string) =>
      request<void>("POST", `/v1/admin/reports/${id}/dismiss`, { outcome, external_reference }),
    decideAppeal: (id: string, reverse: boolean, reason: string) =>
      request<void>("POST", `/v1/admin/decisions/${id}/appeal`, { reverse, reason }),
    stage: (slug: string, stage: AppStage) =>
      request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/stage`, { stage }),
  },
  addToLibrary: (slug: string) => request<void>("PUT", `/v1/me/library/${encodeURIComponent(slug)}`),
  removeFromLibrary: (slug: string) => request<void>("DELETE", `/v1/me/library/${encodeURIComponent(slug)}`),
  desktopRequest: (id: string) =>
    request<{ device_name: string; expires_at: string }>("GET", `/v1/auth/desktop/requests/${encodeURIComponent(id)}`),
  approveDesktop: (id: string) =>
    request<{ redirect_url: string }>("POST", `/v1/auth/desktop/requests/${encodeURIComponent(id)}/approve`),

  providers: () => request<Record<SignInProvider, boolean>>("GET", "/v1/auth/providers"),
  /** The sign-ins linked to the account (crates/slop-api/src/auth/mod.rs). */
  identities: () => request<LinkedIdentity[]>("GET", "/v1/me/identities"),
  unlink: (provider: string, subject: string) =>
    request<void>("DELETE", `/v1/me/identities/${encodeURIComponent(provider)}/${encodeURIComponent(subject)}`),
  /** Combining with the account of a sign-in being linked (auth/merge.rs). */
  combine: {
    preview: () => request<CombinePreview>("GET", "/v1/auth/combine"),
    confirm: () => request<void>("POST", "/v1/auth/combine"),
    cancel: () => request<void>("DELETE", "/v1/auth/combine"),
  },
  /** How to prove you own a repository, and whether you have (repo_owner.rs). */
  repoOwnership: (repo_url: string, repo_token?: RepoTokenInput) =>
    request<RepoOwnership>("POST", "/v1/repos/ownership", { repo_url, repo_token }),
  checkRepoCode: (repo_url: string, repo_token?: RepoTokenInput) =>
    request<RepoOwnership>("POST", "/v1/repos/ownership/check", { repo_url, repo_token }),
  /** A private repository outside GitHub: its read-only token (never shown again once saved). */
  repoToken: {
    status: (slug: string) => request<RepoTokenStatus>("GET", `/v1/apps/${encodeURIComponent(slug)}/repo-token`),
    save: (slug: string, t: RepoTokenInput) =>
      request<RepoTokenStatus>("PUT", `/v1/apps/${encodeURIComponent(slug)}/repo-token`, t),
    forget: (slug: string) => request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/repo-token`),
  },
  /** The newest desktop app build for a platform (for the download button). */
  desktopLatest: (platform: "windows" | "mac" | "linux-deb" | "linux-rpm") =>
    request<{ version: string; url: string }>("GET", `/v1/desktop/latest/${platform}`),
  /** What this server checks, so pages only claim what's true. */
  server: () =>
    request<{ scanning: boolean; maker_signatures_required: boolean; simulation: boolean }>("GET", "/v1/server"),
  ratings: (slug: string) => request<Ratings>("GET", `/v1/apps/${encodeURIComponent(slug)}/ratings`),
  /** The maker's own breakdown by version and platform. */
  ratingsBreakdown: (slug: string) =>
    request<RatingBreakdown>("GET", `/v1/apps/${encodeURIComponent(slug)}/ratings/breakdown`),
  featured: () => request<FeaturedApp[]>("GET", "/v1/featured"),
  vote: (slug: string, vote: "not_slop" | "slop" | null) =>
    request<Ratings>("POST", `/v1/apps/${encodeURIComponent(slug)}/vote`, { vote }),
  forYou: () => request<ForYou>("GET", "/v1/me/for-you"),
  people: {
    list: (q: PeopleQuery = {}) =>
      request<{ people: CreativeCard[]; skills: Array<Skill & { count: number }> }>("GET", `/v1/people${query(q)}`),
    skills: () => request<Skill[]>("GET", "/v1/people/skills"),
    page: (handle: string) => request<CreativePage>("GET", `/v1/people/${encodeURIComponent(handle)}`),
    mine: () => request<MyProfile>("GET", "/v1/me/creative"),
    save: (p: ProfileInput) => request<MyProfile>("PUT", "/v1/me/creative", p),
    portfolio: () => request<PortfolioOverview>("GET", "/v1/me/creative/portfolio"),
    checkPortfolio: (repo_url?: string) =>
      request<PortfolioOverview>("POST", "/v1/me/creative/portfolio/check", repo_url ? { repo_url } : {}),
    setPortfolioOrder: (keys: string[]) => request<PortfolioOverview>("PUT", "/v1/me/creative/portfolio/order", { keys }),
    getPaid: () => request<{ url: string | null; status: string }>("POST", "/v1/me/creative/stripe"),
    refreshPaid: () => request<{ url: string | null; status: string }>("POST", "/v1/me/creative/stripe/refresh"),
    reply: (jobId: string, reply: string) =>
      request<void>("PUT", `/v1/people/reviews/${encodeURIComponent(jobId)}/reply`, { reply }),
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
    ) => request<Job>("POST", `/v1/people/${encodeURIComponent(handle)}/hire`, body),
    jobs: () => request<{ as_client: JobCard[]; as_creative: JobCard[]; waiting_on_you: number }>("GET", "/v1/me/jobs"),
    job: (id: string) => request<Job>("GET", `/v1/jobs/${encodeURIComponent(id)}`),
    step: (id: string, step: string, body: unknown = {}) =>
      request<Job>(
        step === "review" || step === "review/dispute-answer" ? "PUT" : "POST",
        `/v1/jobs/${encodeURIComponent(id)}/${step}`,
        body,
      ),
    record: (id: string) => requestText("GET", `/v1/jobs/${encodeURIComponent(id)}/record`),
    conversationCopy: (id: string) => request<ConversationCopy>("GET", `/v1/jobs/${encodeURIComponent(id)}/conversation`),
    submitCopy: (id: string, copy: ConversationCopy) =>
      request<{ verified_messages: number; unverified_messages: number }>("POST", `/v1/jobs/${encodeURIComponent(id)}/evidence`, { copy }),
    disputeReview: (jobId: string, reason: string, statement: string) =>
      request<void>("POST", `/v1/people/reviews/${encodeURIComponent(jobId)}/dispute`, { reason, statement }),
    pay: (id: string) => request<{ url: string | null; job: Job }>("POST", `/v1/jobs/${encodeURIComponent(id)}/pay`),
    checkPayment: (id: string) => request<Job>("POST", `/v1/jobs/${encodeURIComponent(id)}/payment/check`),
  },
  simUsers: () => request<SimUser[]>("GET", "/v1/sim/users"),
  simLogin: (id: string) => request<void>("POST", `/v1/sim/login/${encodeURIComponent(id)}`),
  /** Full-page navigation target that starts GitHub sign-in. */
  githubSignInUrl: (returnTo: string) => `${API_BASE}/v1/auth/github/start${query({ return_to: returnTo })}`,
  signInUrl: (provider: SignInProvider, returnTo: string) =>
    `${API_BASE}/v1/auth/${provider}/start${query({ return_to: returnTo })}`,
  /** Linking another sign-in: signs in again first if needed, then to the provider. */
  linkUrl: (provider: SignInProvider, returnTo = "/account#sign-ins") =>
    `${API_BASE}/v1/auth/${provider}/link${query({ return_to: returnTo })}`,
  signupInfo: () => request<SignupInfo>("GET", "/v1/auth/signup"),
  signup: (body: { display_name: string; country_code: string; age_confirmed: boolean; accept_terms: boolean }) =>
    request<{ user: Me; return_to: string }>("POST", "/v1/auth/signup", body),
  signout: () => request<void>("POST", "/v1/auth/signout"),
  me: () => request<Me>("GET", "/v1/me"),
  updateMe: (changes: { share_playtime?: boolean; personalize?: boolean; profile_public?: boolean }) => request<void>("PATCH", "/v1/me", changes),
  players: (slug: string) =>
    request<{ playing_now: number; peak_this_month: number; daily_peaks: Array<{ day: string; peak: number }> }>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/players`,
    ),
  reviews: {
    list: (slug: string, vote?: "not_slop" | "slop", offset?: number) =>
      request<{ reviews: AppReview[]; not_slop: number; slop: number; next_offset: number | null }>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/reviews${query({ vote, offset })}`,
      ),
    write: (slug: string, body: string) => request<void>("PUT", `/v1/apps/${encodeURIComponent(slug)}/review`, { body }),
    remove: (slug: string) => request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/review`),
  },
  private: {
    mine: () => request<{ mine: AppSummary[]; shared: Array<AppSummary & { owner: string }> }>("GET", "/v1/me/private"),
    shares: (slug: string) =>
      request<Array<{ user_id: string | null; team: string | null; name: string; shared_at: string }>>(
        "GET",
        `/v1/apps/${encodeURIComponent(slug)}/shares`,
      ),
    share: (slug: string, to: { user?: string; team?: string }) =>
      request<Array<{ user_id: string | null; team: string | null; name: string; shared_at: string }>>(
        "POST",
        `/v1/apps/${encodeURIComponent(slug)}/shares`,
        to,
      ),
    unshareTeam: (slug: string, team: string) =>
      request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/team-shares/${encodeURIComponent(team)}`),
    unshare: (slug: string, userId: string) =>
      request<void>("DELETE", `/v1/apps/${encodeURIComponent(slug)}/shares/${encodeURIComponent(userId)}`),
  },
  push: {
    key: () => request<{ public_key: string }>("GET", "/v1/push/key"),
    subscribe: (subscription: PushSubscriptionJSON) => request<void>("POST", "/v1/me/push", subscription),
    unsubscribe: (endpoint: string) => request<void>("DELETE", "/v1/me/push", { endpoint }),
  },
  friends: {
    list: () => request<Friends>("GET", "/v1/me/friends"),
    /** A profile link or user ID; "friends" when they had already asked you. */
    request: (user: string) => request<{ status: "requested" | "friends" }>("POST", "/v1/me/friends", { user }),
    accept: (id: string) => request<void>("POST", `/v1/me/friends/${encodeURIComponent(id)}/accept`),
    /** Removes a friend, declines their request or cancels yours. */
    remove: (id: string) => request<void>("DELETE", `/v1/me/friends/${encodeURIComponent(id)}`),
    blocks: () => request<FriendPerson[]>("GET", "/v1/me/blocks"),
    block: (id: string) => request<void>("PUT", `/v1/me/blocks/${encodeURIComponent(id)}`),
    unblock: (id: string) => request<void>("DELETE", `/v1/me/blocks/${encodeURIComponent(id)}`),
    connection: (id: string) => request<Connection>("GET", `/v1/users/${encodeURIComponent(id)}/connection`),
    chatSettings: () => request<{ from_anyone: boolean; locked: boolean }>("GET", "/v1/me/chat-settings"),
    setChatSettings: (from_anyone: boolean) =>
      request<{ from_anyone: boolean; locked: boolean }>("PUT", "/v1/me/chat-settings", { from_anyone }),
  },
  teams: {
    list: () => request<TeamCard[]>("GET", "/v1/teams"),
    mine: () => request<{ teams: TeamCard[]; invites: TeamCard[] }>("GET", "/v1/me/teams"),
    create: (body: { slug: string; name: string; kind: "player" | "official"; about: string; join_policy?: string; github_org?: string }) =>
      request<{ slug: string }>("POST", "/v1/teams", body),
    page: (slug: string) => request<TeamPage>("GET", `/v1/teams/${encodeURIComponent(slug)}`),
    update: (slug: string, body: { name?: string; about?: string; join_policy?: string; dns_domain?: string }) =>
      request<void>("PATCH", `/v1/teams/${encodeURIComponent(slug)}`, body),
    join: (slug: string) => request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/join`),
    leave: (slug: string) => request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/leave`),
    invite: (slug: string, user: string) => request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/invites`, { user }),
    accept: (slug: string) => request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/invites/accept`),
    decline: (slug: string) => request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/invites/decline`),
    removeMember: (slug: string, userId: string) =>
      request<void>("DELETE", `/v1/teams/${encodeURIComponent(slug)}/members/${encodeURIComponent(userId)}`),
    setRole: (slug: string, userId: string, role: string) =>
      request<void>("PUT", `/v1/teams/${encodeURIComponent(slug)}/members/${encodeURIComponent(userId)}`, { role }),
    pick: (slug: string, app_slug: string, note: string) =>
      request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/picks`, { app_slug, note }),
    unpick: (slug: string, id: string) => request<void>("DELETE", `/v1/teams/${encodeURIComponent(slug)}/picks/${encodeURIComponent(id)}`),
    addEvent: (slug: string, body: { title: string; starts_at: string; app_slug?: string }) =>
      request<void>("POST", `/v1/teams/${encodeURIComponent(slug)}/events`, body),
    removeEvent: (slug: string, id: string) =>
      request<void>("DELETE", `/v1/teams/${encodeURIComponent(slug)}/events/${encodeURIComponent(id)}`),
    verify: (slug: string) =>
      request<{ verified: boolean; checks: TeamPage["checks"] }>("POST", `/v1/teams/${encodeURIComponent(slug)}/verify`),
  },
  registerBranch: (
    parent: string,
    body: { slug: string; title: string; summary: string; repo_url: string; license_spdx: string; kind: string; note: string; platforms: string[] },
  ) => request<{ slug: string }>("POST", `/v1/apps/${encodeURIComponent(parent)}/branches`, body),
  profile: (id: string) => request<UserProfile>("GET", `/v1/users/${encodeURIComponent(id)}`),
  donations: {
    info: (slug: string) => request<DonateInfo>("GET", `/v1/apps/${encodeURIComponent(slug)}/donate`),
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
      },
    ) => request<{ url: string | null; id: string; done: boolean }>("POST", `/v1/apps/${encodeURIComponent(slug)}/donate`, body),
    check: (id: string) => request<void>("POST", `/v1/me/donations/${encodeURIComponent(id)}/check`),
    mine: () => request<{ donations: MyDonation[]; server_donations: MyServerDonation[] }>("GET", "/v1/me/donations"),
    cancel: (id: string) => request<void>("POST", `/v1/me/server-donations/${encodeURIComponent(id)}/cancel`),
    settings: (slug: string) => request<DonationSettings>("GET", `/v1/apps/${encodeURIComponent(slug)}/donations/settings`),
    saveSettings: (
      slug: string,
      body: { donations_on: boolean; donation_suggested_minor: number; donation_currency: string; runs_servers: boolean; servers_statement: string | null },
    ) => request<DonationSettings>("PUT", `/v1/apps/${encodeURIComponent(slug)}/donations/settings`, body),
    serversShutDown: (slug: string) => request<{ cancelled: number }>("POST", `/v1/apps/${encodeURIComponent(slug)}/servers/shut-down`),
    summary: (slug: string) => request<DonationSummary>("GET", `/v1/apps/${encodeURIComponent(slug)}/donations`),
    refund: (id: string) => request<void>("POST", `/v1/donations/${encodeURIComponent(id)}/refund`),
    account: () => request<DonationAccount>("GET", "/v1/me/donation-account"),
    /** Needs a sign-in from the last few minutes; takes effect after 72 hours. */
    changeAccount: (kind: "switch" | "disconnect", return_to?: string) =>
      request<{ url: string | null; status: string }>("POST", "/v1/me/donation-account/change", { kind, return_to }),
    cancelAccountChange: () => request<void>("POST", "/v1/me/donation-account/change/cancel"),
    challenge: () => request<PowChallenge>("POST", "/v1/donations/challenge"),
    setVat: (vat_number: string | null) =>
      request<{ vat_number: string | null; checked: boolean; note: string | null }>("PUT", "/v1/me/vat", { vat_number }),
    onboard: (body: { tax_notice_ack: boolean; confirm_adult: boolean; return_to?: string }) =>
      request<{ url: string | null; status: string }>("POST", "/v1/me/donation-account", body),
    refreshAccount: () => request<{ url: string | null; status: string }>("POST", "/v1/me/donation-account/refresh"),
    feeStatements: () => request<{ statements: FeeStatement[]; vat: VatTreatment }>("GET", "/v1/me/fee-statements"),
  },
  interested: {
    mine: () => request<Array<AppSummary & { interested_at: string }>>("GET", "/v1/me/interested"),
    add: (slug: string) => request<void>("PUT", `/v1/me/interested/${encodeURIComponent(slug)}`),
    remove: (slug: string) => request<void>("DELETE", `/v1/me/interested/${encodeURIComponent(slug)}`),
    /** Upcoming apps for this viewer, each with why; the most anticipated when not personal. */
    forYou: () => request<Array<AppSummary & { reason: string }>>("GET", "/v1/upcoming/for-you"),
  },
  notifications: {
    list: () => request<{ unread: number; items: Notification[] }>("GET", "/v1/me/notifications"),
    read: (ids: string[] = []) => request<void>("POST", "/v1/me/notifications/read", { ids }),
    prefs: () => request<EmailPrefs>("GET", "/v1/me/email-prefs"),
    setPrefs: (changes: Partial<Record<Notification["category"], boolean>>) =>
      request<EmailPrefs>("PUT", "/v1/me/email-prefs", changes),
  },
  legal: {
    index: () => request<LegalIndexEntry[]>("GET", "/v1/legal"),
    document: (kind: string, version?: string) =>
      request<LegalDocument>("GET", `/v1/legal/${encodeURIComponent(kind)}${query({ version })}`),
    notices: () => request<LegalNotice[]>("GET", "/v1/me/legal"),
    accept: (kind: LegalKind, version: string) => request<void>("POST", "/v1/me/legal/accept", { kind, version }),
  },
  reports: {
    reasons: (kind: ReportKind) => request<ReportReason[]>("GET", `/v1/reports/reasons${query({ kind })}`),
    send: (body: ReportBody) => request<{ reference: string; due_at: string }>("POST", "/v1/reports", body),
    mine: () => request<MyReport[]>("GET", "/v1/me/reports"),
  },
  decisions: {
    mine: () => request<ModerationDecision[]>("GET", "/v1/me/decisions"),
    appeal: (id: string, appeal: string, counter_notice?: CounterNotice) =>
      request<ModerationDecision>("POST", `/v1/me/decisions/${encodeURIComponent(id)}/appeal`, { appeal, counter_notice }),
  },
  /** Everything linked to the account, as JSON text to save. */
  exportData: () => requestText("GET", "/v1/me/export"),
  closeAccount: (confirm: string) => request<void>("POST", "/v1/me/close", { confirm }),
  sessions: () => request<SessionInfo[]>("GET", "/v1/me/sessions"),
  revokeSession: (id: string) => request<void>("DELETE", `/v1/me/sessions/${encodeURIComponent(id)}`),
};
