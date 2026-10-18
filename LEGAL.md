# Slop — Legal Framework

**Status:** Draft for review by a lawyer. **This is not legal advice.**
**Last updated:** 2026-10-17 · **Company:** IronPoint Development LLC (US)
**Related:** [`DESIGN.md`](DESIGN.md) · [`PAYMENTS.md`](PAYMENTS.md) · [`SECURITY.md`](SECURITY.md) · [`ARCHITECTURE.md`](ARCHITECTURE.md) · [`METRICS.md`](METRICS.md)

---

## 0. How to read this

This document sets out the **legal positions Slop takes** and the **design rules that make those positions true**. It gives a lawyer one place to check them, and gives the team a list of rules not to break.

> **The one thing that matters most:** courts, regulators, tax offices and card networks look at **what actually happens**, not at what we call it. Calling a payment a "donation" doesn't make it one. It's a donation because **nothing is given in return**.
>
> Every position below holds only as long as the matching **Design rule** is followed. If a design rule changes (for example, donors get a perk), the legal analysis changes too. Check with the lawyer first.

Each section is set out as: **Position**, then **Design rules** (what the product must do), then **⚖️ For the lawyer** (what to confirm).

---

## 1. Summary of positions

| # | Position | Held up by |
|---|---|---|
| 1 | Slop is a **directory and delivery service** for free, open-source software. It doesn't sell software. | All downloads are free. The licence is the maker's open-source licence, between the maker and the user. |
| 2 | **Donations are voluntary gifts from players to makers.** They're not purchases, and Slop isn't a party to them. | Nothing is unlocked or given in return. Apps can't tell who has donated. |
| 3 | **Slop never holds, receives or moves anyone else's money.** | Every payment is a direct charge into the **maker's or creative's own Stripe account**. Slop receives only its own platform fee. No balances, wallets, credits or escrow. |
| 4 | **Humans is the only place where something is sold:** a creative's services to a client. The contract is between those two. Slop is the marketplace, not the seller, employer or judge. | The creative sets the price and terms and is paid directly. Slop doesn't control the work and doesn't decide disputes. |
| 5 | Slop is a **hosting provider / online platform** for content its users post. | Notice-and-action, DMCA process, repeat-infringer policy, statements of reasons. |
| 6 | **Security scanning is a voluntary safety measure, not a guarantee.** | Careful wording ("scanned", never "safe"), public scan reports, a clear disclaimer. |
| 7 | **"Slop" / "Not Slop" votes are users' opinions,** not statements by Slop. | Only people who have launched the app can vote. Votes are shown as user ratings. |

---

## 2. The company

**Position:** Slop is operated by **IronPoint Development LLC**, a US limited-liability company, so its owners aren't personally liable.

**Design rules**
- Every contract, Stripe account, cloud account and domain is in the **company's** name, not a person's.
- Insurance:
  - **Technology errors & omissions (E&O):** covers claims that the service failed, e.g. malware getting past a scan.
  - **Cyber insurance:** covers data breaches.

**⚖️ For the lawyer**
- **The company is IronPoint Development LLC, based in the US** (decided 2026-10-17). Slop is its product.
- **Formed in Wyoming** (decided 2026-10-17). What that means:
  - A **Wyoming registered agent** is required.
  - There's a small **annual report** with a licence tax based on Wyoming assets, with a low minimum.
  - Wyoming has **no state income tax**. Federal tax still applies, and states where IronPoint has staff or offices may also tax it.
  - **Terms of Service:** choose Wyoming law and Wyoming courts (or arbitration) for US users. **EU and UK consumers keep their own country's consumer protections** whatever the Terms say, so the Terms must say that too.
  - ⚖️ Does IronPoint need to register as a foreign LLC in the state where the founders actually work?
- Does Slop need a **"doing business as" (DBA) registration** for the name "Slop"?
- **Slop launches in the US, the EU and the UK** (decided 2026-10-17). As a company with no office in the EU or UK, it needs:
  - an **EU representative** for the GDPR (Art. 27), and a **UK representative** for the UK GDPR (Art. 27);
  - a **legal representative** under the EU Digital Services Act (Art. 13).

  These are paid services, around a few hundred to a few thousand dollars a year each. Some firms cover all three.
- **US basics:**
  - Registered agent.
  - Federal tax ID (EIN).
  - Separate bank account.
  - **DMCA designated agent** (§5.1).
  - **State privacy laws:** for example, California's CCPA applies once Slop passes its thresholds, such as handling data on 100,000+ California residents a year.

---

## 3. Money

### 3.1 How the money moves

```
 Player (donor)                     Stripe                                  Maker
 ──────────────  pays D + fees  →  charge on the MAKER's own Stripe   →   maker's bank
                                   account (maker = merchant)
                                          │
                                          └── application fee (5% of D) ──→ Slop's Stripe account
                                                                             (Slop's own revenue)
```

**Position:** Slop never has possession or control of donors' or makers' money. Stripe is the licensed payment provider. The maker is the recipient and the merchant on the card statement. Slop's only income is its **platform fee**, which Stripe deducts automatically.

**Design rules**
- Use **Stripe Connect** with accounts where the maker has their **own Stripe account and dashboard**, and **Stripe, not Slop, carries losses** (what Stripe calls "Standard" accounts). Use **direct charges** with an **application fee**.
- Slop **never**:
  - keeps balances for anyone, or offers top-ups, wallets, store credit or gift cards;
  - pools funds, or pays makers out of its own account;
  - moves money between makers;
  - promises refunds on a maker's behalf;
  - holds money until something happens (no escrow).
- The card statement shows the **maker's** name (Stripe's statement descriptor on their account), with "via Slop" if Stripe allows it.

**Precedents the lawyer can compare:** creator-support platforms that send money directly to the creator's own Stripe/PayPal account (e.g. Ko-fi), and itch.io's "direct to you" payment mode.

**⚖️ For the lawyer**
- Does this structure keep IronPoint outside **US federal (FinCEN) and state money-transmitter licensing**? Relevant points:
  - Slop never receives others' funds.
  - Several states recognise an "agent of the payee" exemption.
  - Stripe is the licensed entity.
- If Slop serves the EU: does it also stay outside payment-institution / e-money licensing (PSD2 / EMD2)?
- What does Stripe's Connect agreement say about platform responsibilities for these account types: KYC, negative balances, prohibited businesses?

### 3.2 Donations are not purchases
**Position:** a donation is a voluntary payment from a player to a maker, with nothing given in return. It isn't a sale of goods, digital content or services. So purchase-related rules (sales tax/VAT on sales, consumer-sale rights, marketplace-seller rules) shouldn't apply to Slop.

**Design rules**
1. **Nothing in return, ever** (decided 2026-10-17). A donation never unlocks content, features, items, early access, server access, ranks, badges, credits, supporter lists, roles, or in-game anything.
   - All a donor gets is a thank-you message and a receipt.
   - **Donors are never listed publicly.** A donor may choose to share their name and a message **privately with the maker**.
2. **Apps can't know who donated.** Slop offers no API, SDK call, token or webhook that tells an app whether a player donated. A maker *can't* tie features to donations even if they wanted to.
3. **The app is the same for everyone.** The scan checks that the app's code has no donation checks and no references to "supporter" features.
4. **Free first.** "Add to library" and "Install" are never behind the Donate button, and the store page never suggests that donating is expected.
5. **Wording:** follow the word list in §12. Never "buy", "purchase", "price", "order" or "customer" for donations.
6. **Tax notice to makers:** "Donations may be taxable income for you. Slop doesn't give tax advice." Shown to makers when they set up donations.
7. **No refunds owed.** Donations are final. Mistakes (e.g. a wrong amount) can be refunded by the **maker**, through a button Slop provides. Fraud is handled by Stripe.

**⚖️ For the lawyer**
- Is a donation with nothing in return outside VAT/sales-tax scope in our main markets?
- Do any countries treat "donation" as a word reserved for charities? In German, "Spende" suggests a charity. Would "tip" or "support" be safer there?
- Are executed small gifts fine under contract law (e.g. Germany §516 BGB)?

### 3.3 Server donations (recurring)
**Position:** a server donation is a **recurring voluntary donation** to help pay for a game's or app's servers. It isn't a subscription, because it doesn't buy access to anything.

**Design rules**
- Only offered on apps whose publisher **declares that they run servers**, with a short statement of what the money helps pay for.
- **Server access is never tied to donating.** The same rule 2 from §3.2 applies: the app can't know who donates.
- Set up as a monthly donation on the **maker's** Stripe account (Stripe Billing on the connected account, with Slop's application fee).
- Before the donor confirms, they're told: the monthly amount, that it repeats every month until cancelled, and how to cancel.
- **Cancel online in two clicks** at any time, from the library or account page, with no reason required. A receipt is sent for every payment.
- Slop **doesn't promise** that servers stay online. The maker's statement is theirs. If a maker shuts down the servers, active server donations are cancelled automatically and donors are told.

**⚖️ For the lawyer**
- Do automatic-renewal laws apply even though nothing is bought? Examples: California's Automatic Renewal Law and other US state laws, and the EU/German rules on online cancellation (§312k BGB). The design above aims to comply either way.

### 3.4 Donors pay the card fees
**Position:** the donor chooses how much the maker receives, and sees the total they'll pay, including payment-processing costs, **before** paying. This is a single, all-in price for the donation, not a surcharge for using a card.

**Design rules**
- Show one total: "You pay **$3.40** · @maker receives **$2.85** ($3.00 minus Slop's 5%) · $0.40 covers card processing."
- The fee addition is the **same no matter which payment method** is used (card, wallet, bank). If it varied by method, it could count as a surcharge.
- It's not an optional add-on box that's ticked in advance (pre-ticked boxes for extra payments aren't allowed in the EU). It's part of the total shown.

**⚖️ For the lawyer**
- Is this a surcharge under EU PSD2 Art. 62(4) (surcharges on consumer cards are banned in the EU), US state surcharge laws, or Visa/Mastercard rules?
- If it is a surcharge, the fallback is: the donor chooses the total, and fees come out of what the maker receives.

### 3.5 Slop's 5% fee
**Position:** Slop's fee is a charge **to the maker** (or creative) for using the platform. It's Slop's own revenue. It isn't a cut of a sale, because nothing was sold.

**Design rules**
- The fee is described in the Publisher Agreement and the Humans Terms. Makers get a monthly fee statement/invoice from Slop.
- Slop's fee revenue goes into Slop's own accounting.

**VAT (EU and UK) — decided 2026-10-17: VAT comes out of Slop's fee, never out of the donation.**
- **The donation itself has no VAT.** Nothing is given in return, so it's outside VAT (position 2). Adding VAT to a donation would treat it as a sale, which would undermine our main position. So **never** put VAT on, or take it from, the donation.
- **Slop's 5% fee is a service Slop sells to the maker**, so VAT may apply to that:
  - **Makers who are VAT-registered businesses** (they give a valid VAT number): no VAT charged by Slop. The maker accounts for it under the reverse charge.
  - **Makers who are individuals in the EU/UK:** Slop owes VAT at the maker's country rate (EU 17–27%, UK 20%).
- **Design rule: the 5% includes VAT.** The headline stays "5%". When VAT is due, Slop pays it out of its own 5%. For example, at the UK's 20%, Slop keeps about 4.17% and pays about 0.83% to HMRC. Makers' fee statements show the VAT.
- **Registrations:**
  - **EU:** one registration under the **non-Union OSS** scheme covers all 27 countries, with quarterly returns.
  - **UK:** register with HMRC as a **non-established taxable person**. There's **no minimum threshold** for non-UK businesses, so this is needed from the first fee.
- **Proving where a maker is:** keep two pieces of evidence that agree, for example billing country from Stripe and IP address/country. Stripe Tax can do the calculation.
- **US:** some states charge sales tax on digital/SaaS services, which may include Slop's fee (§15).
- **Humans:** VAT on a creative's own services is the creative's responsibility. Slop's fee on Humans jobs follows the rules above.

**⚖️ For the lawyer / accountant**
- Confirm donations are outside VAT in the EU and UK.
- Confirm Slop's fee is an "electronically supplied service" for VAT.
- Confirm Slop isn't treated as the supplier of Humans services. Commissioned work is mostly human effort, which normally keeps platforms out of the "deemed supplier" rule.

### 3.6 Splitting donations between several people
**Position (decided 2026-10-17):** each app's donations go into **one** Stripe account, chosen by the publisher. If several people made the app, **the publisher splits the money with them outside Slop**. Slop isn't involved in or responsible for that split.

**Why:** splitting one payment between several accounts would mean the money first lands in **Slop's** platform account. That breaks position 3: Slop would be in the flow of funds and liable for chargebacks.

**Design rule:** the Publisher Agreement says the publisher is solely responsible for paying anyone they owe a share to.

### 3.7 Who handles what: Stripe, Slop, the maker
| Task | Who |
|---|---|
| Checking makers' identity (KYC), sanctions screening of payments | **Stripe** |
| Card data and PCI DSS compliance | **Stripe** (Slop uses Stripe Checkout/Elements, which keeps Slop on PCI SAQ A) |
| Chargebacks and refunds (money side) | **Stripe and the maker's account**. Slop keeps no reserve and isn't liable. |
| US tax forms (1099-K) | **Stripe**, for these account types ⚖️ confirm |
| Which countries makers can receive donations in | **Stripe's** list of supported countries. Makers elsewhere can list apps but can't receive donations. |
| Blocking accounts from comprehensively sanctioned regions | **Slop** (at sign-up), as well as Stripe |
| Paying tax on donations received | **The maker** |

### 3.8 In-app purchases
**Position (decided 2026-10-17):** **no in-app purchases.** Apps on Slop can't sell anything: no items, supporter packs, subscriptions, paid access or currencies. The monetization scan rejects any app that tries. This keeps Slop entirely outside the sale of digital goods. Revisit only with the lawyer.

---

## 4. Humans: the one real sale

### 4.1 Roles
**Position:**
- The **creative** sells services to the **client** (a developer).
- The **service contract** is between them.
- Slop provides the directory, the messaging and the payment page. It's an **intermediary**, not a party.
- Slop is **not the employer** of creatives and **doesn't decide disputes**.

**Design rules**
- The creative sets their own **prices, scope, schedule, methods and whether to accept a job**. Slop never sets or limits prices, assigns work, monitors how work is done, or ranks creatives by acceptance rate. (This control test matters for employment status; see §4.4.)
- Payment is a **direct charge on the creative's Stripe account**, with Slop's 5% application fee, like donations. The creative is the merchant.
- Before the client pays, the payment page states:
  - "Your contract is with @creative, not Slop."
  - "Slop doesn't settle disputes."
  - Who to contact.
  - That their rights are against the creative.
- **Default Service Agreement:** a short template that both sides accept, and can change. It covers:
  - Scope and deliverables.
  - Price.
  - When payment is due (up front, on delivery, or by milestone), as the creative chooses.
  - Delivery date.
  - Revisions.
  - **Who owns the result:** a transfer of rights, or a licence.
  - The creative's promise that their **AI-use disclosure** is true, and that they own or have the rights to everything they deliver.
  - The refund policy.
  - Which law applies to the contract.
- **Completion:** after delivery, the client has **7 days** to mark the job complete. If they don't, it's marked complete automatically, and voting opens.
- **Refunds:** issued by the **creative**, using a button Slop provides. If they refuse, the client's route is their **card issuer (chargeback)** or the courts. Slop says this openly.
- Slop **does** act on **breaches of its own rules**, such as scams, non-delivery as a pattern, fraud or illegal content. It can suspend or remove accounts, but it doesn't rule on who's right in a disagreement over the work.

**⚖️ For the lawyer**
- Is "Slop doesn't settle disputes" enforceable towards consumers? Some clients will be individuals, not businesses.
- Do we need an **ODR / dispute-resolution notice**? Germany's VSBG requires saying whether you take part in consumer arbitration; the EU ODR platform closed in 2025.

### 4.2 Platform duties under the EU Digital Services Act
**Position:** Slop is an online platform. Humans also makes it an online marketplace. While Slop is a **micro or small enterprise** (fewer than 50 staff and €10M or less in turnover), it's exempt from the extra DSA obligations for online platforms and marketplaces (Arts. 19 and 29). It still has to meet the basic duties for hosting services.

**Design rules** (comply anyway, it's cheap):
- Points of contact (Arts. 11–12).
- Clear terms (Art. 14).
- **Notice-and-action** reporting form (Art. 16).
- A **statement of reasons** for every removal or restriction (Art. 17).
- Collect creatives' business identity through Stripe onboarding. This is ready for the marketplace rule on knowing your traders (Art. 30) if Slop grows beyond the exemption.

### 4.3 Reporting creatives' income (DAC7, US 1099-K)
**Position:** Humans jobs are "personal services" under the EU's DAC7 rules. **Slop, as the platform operator, may have to report** creatives' income to tax authorities. Donations to software makers probably don't fall under DAC7 (software isn't "goods", and donations aren't services) ⚖️.

**Design rules**
- For creatives with EU links, collect during onboarding: legal name, address, tax ID, date of birth, and bank details (Stripe holds these).
- An annual DAC7 report by 31 January.
- Slop is outside the EU, so it registers in one EU country to report.
- **UK:** the UK has its own version, the Reporting Rules for Digital Platforms (in force since 1 January 2024). Same data, reported to HMRC each January.

### 4.4 Employment status
**Position:** creatives are independent businesses, not workers. This includes under the **EU Platform Work Directive** (2024/2831), which EU countries must turn into national law by **2 December 2026**. That directive presumes employment where a platform *directs and controls* the work. Slop doesn't.

**Design rules:** the "no control" rules in §4.1. Never add things like required response times, Slop-set prices, penalties, or automated ratings that limit who gets work.

### 4.5 Rating real people
**Position:** 👍/👎 on Humans profiles are opinions from verified clients about a service.

**Design rules**
- Only clients who completed a job can vote.
- The creative can **reply publicly**.
- Reviews with insults, personal attacks or private information are removed.
- Creatives can report reviews.

**⚖️ For the lawyer:** defamation risk, and the GDPR (ratings of identifiable people are personal data, so we need a lawful basis and a right to object).

### 4.6 Chat (planned)
**Position (decided 2026-10-17):** chat is **between people**: friends, team members, creatives and clients. **Slop doesn't read messages, and can't:** chat is **end-to-end encrypted** (decided).

**Design rules**
- **No automatic reading or scanning of message content** on Slop's servers.
  - Link warnings ("this link leaves Slop") run **inside the recipient's app**. Nothing is sent to Slop to check.
- Moderators see a message **only when a participant reports it**. The report sends just the reported messages, with the reporter's consent.
- **End-to-end encryption** (decided), so that Slop *can't* read messages, not just *doesn't*. That's the strongest evidence for this position. Reports still work, because the reporter's app sends the decrypted messages it already has. Technical design: `SECURITY.md` §8.5.
- **Law-enforcement requests:** a written policy. In the US, the Stored Communications Act controls what a provider may disclose, and when a warrant is needed.
- **Abuse material:** if Slop *becomes aware* of it (through a report), it must report it (§5.3). Without reading messages, Slop only knows what users report.

- **UK Online Safety Act:** chat makes Slop a "user-to-user service". Encrypted chat is allowed, but Slop still needs:
  - a risk assessment that covers chat;
  - working reporting and blocking tools;
  - a plan for responding to Ofcom.

  The Act lets Ofcom, in limited cases, require scanning for child abuse material even in private messages. The government has said it won't use that power until it's technically feasible. This is a known tension with end-to-end encryption, not a blocker today (§5.9).

**⚖️ For the lawyer:**
- Retention period for message metadata.
- EU ePrivacy rules for chat.
- Whether Slop must preserve metadata on request (US law-enforcement preservation requests, 18 U.S.C. §2703(f)).
- The Online Safety Act position on encrypted chat.

---

## 5. Content, listings and liability

### 5.1 Safe harbours for user content
**Position:** Slop hosts content posted by others: apps, branches, store pages, reviews, votes, profiles, messages. It's not liable for that content unless it knows it's illegal and does nothing.

**Design rules**
- **US:**
  - Section 230 (47 U.S.C. §230) covers user content and moderation done in good faith.
  - **Maker-hosted builds** (`ARCHITECTURE.md` §6.4) put Slop closer to a **directory that links** to the maker's files than to a host of them. That fits position 1, and the DMCA's protection for "information location tools" (§512(d)) as well as for hosting (§512(c)). ⚖️ Confirm.
  - **DMCA (17 U.S.C. §512):** register a **designated agent** with the US Copyright Office (renewed every 3 years), publish a takedown and counter-notice process, and enforce a **repeat-infringer policy**.
- **EU:**
  - Hosting safe harbour (DSA Art. 6).
  - **Art. 7** says voluntary checks such as our scans **don't remove** the safe harbour. Our scanning helps our position rather than hurting it.
- Act quickly on valid notices. **Keep records** of every notice, decision, reason and time taken (§13).
- **Terrorist content:** EU removal orders must be acted on **within 1 hour** (Regulation 2021/784). The on-call process must cover this.

### 5.2 Security scanning: what we promise
**Position:** scanning reduces risk but **can't guarantee** that software is safe. The maker is responsible for their software. Slop is responsible only for running its process with reasonable care.

**Design rules**
- **Wording:**
  - Use "Scanned", "Passed Slop's checks" and "See the security report".
  - **Never** "safe", "virus-free", "guaranteed", "verified secure" or "100%". (Exaggerated safety claims are a misleading-advertising risk: FTC Act §5 in the US, unfair commercial practices rules in the EU.)
- Every store page links to a public **"What scanning means"** page explaining the limits.
- The Terms limit Slop's liability as far as the law allows. In EU and German consumer law, liability for **intent and gross negligence** and for personal injury **can't be excluded**. So the strongest protection is to **actually run the process carefully and keep records of it** (§13).
- When a malicious app is found: the kill switch (`SECURITY.md` §5.5), a notice to users, and a record of the action.

### 5.3 Illegal content
- **Child sexual abuse material:**
  - Hash matching on uploads.
  - US providers must report what they find to NCMEC (18 U.S.C. §2258A).
  - Under EU and national law, report it and keep evidence as required.
- **Banned AI uses:**
  - Apps that do things the EU AI Act prohibits (Art. 5) aren't allowed.
  - Makers must meet the AI Act's **transparency duties** (Art. 50, in force since 2 August 2026): tell users they're talking to an AI, and label AI-generated or altered content where required. The **Publisher Agreement** places this duty on makers.
- **Loot boxes and gambling:** banned in our rules (`DESIGN.md` §12). This also avoids gambling and loot-box laws (e.g. Belgium).

### 5.4 Open-source licences and AI-made code
**Position:** makers promise that they have the right to publish everything in their app under the licence they choose. Slop relies on that promise and on its licence checks.

**Design rules**
- **Publisher Agreement:**
  - The maker promises they own or are licensed to use all code and assets, including AI output.
  - They'll comply with all third-party licences.
  - They'll protect Slop against claims caused by their app (an indemnity).
- **GPL and similar licences:** when Slop distributes binaries, the matching source has to be available. Public repos meet this. For Private Slop with private repos, the **owner** is responsible.
  - Slop **doesn't keep copies** of makers' builds or source (decided 2026-10-17, `SECURITY.md` §12.2). Public apps rely on the public repo, and Private Slop is shared only with people the owner chooses.
- **AI and copyright:** in the US, purely AI-generated material generally **can't be protected by copyright** (US Copyright Office guidance; *Thaler v. Perlmutter*, D.C. Cir. 2025). Most EU countries take a similar view.
  - **The UK is different:** it protects "computer-generated works" (Copyright, Designs and Patents Act 1988, s.9(3)), with the person who made the arrangements treated as the author. The government has consulted on changing this.
  - Makers' licences still cover the parts made by people.
  - Slop doesn't promise that anyone owns the copyright, and the Terms say so.
  - AI tools can also reproduce code they learned from, which is what the licence-matching scan is for.

### 5.5 Votes ("Slop" / "Not Slop")
**Position:** the labels show users' opinions, added up.

**Design rules**
- Labels always read as ratings from users ("Mostly Slop — 34% of 1,204 players"), never as Slop's own judgement.
- Show weighted and unweighted results (`DESIGN.md` §7).
- Fake-vote rules are enforced.
- Makers can report rating manipulation.

### 5.6 Names and trademarks
- Check that the name and logo "**Slop**" are free to use as a trademark, and register them, in the main markets.
- **Don't copy Steam's** name, logo, colours or distinctive look. "Like Steam" is fine in descriptions, but not in branding.
- Makers' trademark complaints use the same notice process.

### 5.7 Age ratings for games
- Games can carry ratings for violence, sexual content and so on.
- ⚖️ If Slop offers games in **Germany**, youth-protection law (JMStV) needs ratings, plus age checks for adult-only content.
- Joining **IARC** (the international rating system used by Google Play and Microsoft Store) is an option ⚖️.
- Until then, makers rate their games themselves with an IARC-style questionnaire, and content rated 18+ is hidden from accounts under 18.

---

### 5.8 AI and software laws in the EU and UK

**The key point:** these laws regulate **AI systems** (software that *uses* AI when it runs), not software that was *written with* AI.
- An AI-made game with no AI features inside it isn't covered by the EU AI Act.
- An app that **contains** a chatbot, an image generator or similar *is* covered, and its **maker** is the "provider" with the duties.

**EU AI Act** (Regulation (EU) 2024/1689)
| What | Since | What it means for Slop |
|---|---|---|
| **Banned practices** (Art. 5): manipulative or deceptive techniques, exploiting vulnerabilities (age, disability), social scoring, emotion recognition at work or school, scraping faces to build recognition databases, biometric guessing of sensitive traits, and a few police uses | 2 Feb 2025 | **Banned on Slop.** Content Policy, Publisher Agreement, and review on report. Fines go up to €35M or 7% of turnover, so this must be strict. |
| **AI literacy** (Art. 4): providers and deployers make sure their staff understand the AI they use | 2 Feb 2025 | Makers with AI features. Slop itself, if it uses AI tools in moderation or scanning. |
| **General-purpose AI models** (foundation models) | 2 Aug 2025 | Only companies that *train* such models. Not Slop, and rarely makers. |
| **Transparency** (Art. 50): tell people when they're talking to an AI; mark AI-generated images, audio and video as machine-readable; label deepfakes | 2 Aug 2026 | **Makers** of apps with these features. Slop's Publisher Agreement requires it, and the store page has an "AI features" field. |
| **"High-risk" AI** (Annex III): hiring, exams and grading, credit scoring, biometric identification, critical infrastructure and similar | 2 Aug 2026 (product-safety uses: 2027) | **Not allowed on Slop.** Distributing high-risk systems brings duties for distributors (Art. 24), such as checking CE marking. Banning them is simpler. |

⚖️ In November 2025 the EU proposed a "Digital Omnibus" that would **delay** the high-risk rules and soften AI literacy. Check whether it was adopted, and what dates now apply.

**Other EU rules that hit software distribution** (not AI laws, but relevant):
- **Cyber Resilience Act** (Regulation (EU) 2024/2847): security requirements for "products with digital elements".
  - **Vulnerability and incident reporting** duties for manufacturers applied from **11 September 2026**. Everything else applies from **11 December 2027**.
  - Free open-source software that isn't supplied as a **commercial activity** is largely outside it. The CRA's recitals say **accepting donations without intending to make a profit** doesn't by itself make it commercial.
  - This supports the donations-not-sales position too. ⚖️ Where's the line for a maker who lives off donations, and what are Slop's duties as a distributor?
- **New Product Liability Directive** (EU) 2024/2853: treats **software as a product** for defects that cause harm (including destroyed data). It applies to products placed on the market from **9 December 2026**.
  - It excludes free open-source software supplied outside a commercial activity.
  - Online platforms can be liable like a distributor if they present a product as their own, or can't name the maker. Slop always names the maker and links their repo, so it shouldn't fall into this. ⚖️
- **GDPR Art. 22** (automated decisions): rejecting a maker's app purely through automatic scanning is an automated decision. Slop offers **human review on request** for every block or rejection. That also meets the DSA's complaint rules.

**UK**
- **No AI Act.** The UK regulates AI through existing regulators (the ICO for data, the CMA for competition and consumers, Ofcom for online safety). An AI bill has been discussed but hasn't been passed. ⚖️ Check its status.
- **Data (Use and Access) Act 2025** relaxed the UK GDPR's rules on automated decisions, but human review is still expected for significant decisions. Slop's design already has it.
- **Copyright:** computer-generated works are protected (see §5.4).
- **Online Safety Act** and **consumer law**: see §5.9 and §5.10. These matter more for Slop than AI law in the UK.

### 5.9 UK Online Safety Act 2023
Slop is a **"user-to-user service"** with UK users: reviews, profiles, Humans listings, apps, and chat. Ofcom enforces it.
- **Illegal-content duties** (enforced since 17 March 2025):
  - a written **illegal content risk assessment**, kept up to date;
  - measures in proportion to the risk;
  - easy **reporting and complaints**;
  - a named person accountable for compliance.
- **Children:** under the Act, a "child" is anyone **under 18**. UK users aged 16–17 can use Slop (§6, decided 2026-10-17), so children *are* likely to access it. Slop therefore needs:
  - a **children's access assessment** and a **children's risk assessment**, written down and kept up to date;
  - **child-safety measures** in line with Ofcom's Protection of Children Codes (enforced since 25 July 2025). For Slop these are mainly:
    - the **under-18 view by default** for every UK account, with 18+ content hidden;
    - **highly effective age checks** to unlock 18+ content;
    - **safe contact defaults** (only connections can message under-18s, and they aren't suggested to strangers);
    - easy reporting and blocking;
    - moderation of content that's harmful to children.
  - **Sexually explicit content is banned** (§5.11). That keeps Slop out of the strictest age-check rules for pornography ("primary priority content"). The age check is only needed to unlock 18+ (mainly violent or mature) games.
- **Size matters:** most of the heaviest duties fall on large or high-risk services. Ofcom's codes scale with size and risk. ⚖️ Which codes apply to a small service like Slop?

### 5.10 Reviews and prices: EU and UK consumer rules
- **Reviews:**
  - **EU** (Omnibus Directive): a platform that shows reviews must **say whether and how it checks that they come from real users**. Slop's rule, "only people who launched the app through Slop can vote", is exactly that. The "How ratings work" page must say so, along with the weighting.
  - **UK** (Digital Markets, Competition and Consumers Act 2024, in force April 2025): **fake reviews are banned**, and platforms must take reasonable steps to prevent and remove them. Slop's vote rules and abuse checks (`SECURITY.md` §11) are those steps. Keep records (§13).
- **Prices:**
  - **UK:** the same Act bans **"drip pricing"**: unavoidable fees must be included in the first price shown.
  - **Design rule:** wherever a donation amount appears with a total, show the **total the donor pays** (including card processing) right there, never added at the last step. The mockup's donate box already shows the full breakdown before paying.

### 5.11 Sexual content (banned)
**Position (decided 2026-10-17):** **sexually explicit content isn't allowed on Slop**: in apps, store pages, screenshots, reviews, profiles, Humans listings or Private Slop.

**What counts** (proposed definition for the Content Policy):
- **Banned:**
  - depictions of sexual acts;
  - nudity with a sexual focus;
  - sexually explicit text, including in games;
  - content whose main purpose is sexual arousal;
  - "adult" versions, unlock codes or patches that add any of the above.
- **Allowed, with a content label (§5.7):**
  - romance, kissing and suggestive themes that aren't explicit;
  - non-sexual nudity in an artistic, medical or educational context.

  ⚖️ The lawyer should help draw this line clearly, because unclear rules are hard to enforce fairly.

**Why:**
- Stripe's terms prohibit adult content, and a problem there would cut off donations for every maker.
- It avoids the strictest age-check laws for pornography:
  - UK Online Safety Act "primary priority content";
  - age-verification rules in Germany, France and Italy;
  - US state laws (*Free Speech Coalition v. Paxton*, 2025).
- Steam and itch.io removed many adult games in 2025 under payment-processor pressure.

**Design rules:**
- **Publisher Agreement and Content Policy** state the ban.
- The **content scan** checks images (and text where feasible). Matches are **rejected**, with a reason and a human review on request (§5.8, GDPR Art. 22).
- **Reports** for sexual content are a standard report reason.
- **Enforcement:** first violation, the app is removed and the maker warned. Repeat violations lose publishing rights. Anything involving minors means an **immediate ban and a report** (§5.3).

**Still required even with the ban** (because users can still try to post it):
- **Anything sexual involving minors**, including drawn or AI-generated depictions, is illegal (UK Coroners and Justice Act 2009; 18 U.S.C. §1466A in the US; similar laws in the EU). Zero tolerance, with reporting where required (§5.3).
- **Non-consensual intimate images and deepfakes:** the US **TAKE IT DOWN Act** (2025) requires a request process and **removal within 48 hours** of a valid request, with platform compliance due by May 2026. The UK criminalises creating and sharing sexual deepfakes. Slop needs the request form and the 48-hour process.

## 6. Accounts and age

**Position (decided 2026-10-17):**
- **Accounts are 16+ everywhere,** including the UK.
  - 16 is the highest age the GDPR allows for children's own consent (Art. 8), so it works in every EU country.
  - It's above the US COPPA threshold (13).
- **UK users under 18 aren't blocked. 18+ content is hidden from them.** In the UK every account **starts in the under-18 view** and **unlocks 18+ content only after a real age check**.
- **Donations and Humans jobs require 18+ everywhere** (the age at which someone can make binding contracts). Users confirm this when they first donate or hire.

**Design rules: finding UK users**
- At sign-up and sign-in, Slop works out the user's country from:
  - IP address country;
  - Stripe billing country, if they pay;
  - the phone's app-store country or device region;
  - the country the user states.

  **If any signal says UK, the UK rules apply.**
- **Store only the country,** never a precise location or the IP history.
- **No VPN detection** (decided). Slop uses the signals it has and doesn't try to catch VPNs.

**Design rules: the UK under-18 view (the default for every UK account)**
- **18+ content is hidden:** games rated 18+ (§5.7), and 18+ reviews, screenshots and store-page content.
- **Safe defaults for chat and contact** (Ofcom's codes for protecting children). In the under-18 view:
  - only **friends, team members and Humans clients/creatives** the user already has a job with can message them. The "anyone can message me" setting is **turned off and locked**;
  - the profile **isn't suggested** to strangers ("people you may know", search);
  - **block and report** are always one tap away. Reports about contact with a child are handled first.
- Reviews, profiles and Humans listings are moderated for content that's harmful to children (bullying, hate, violence, dangerous challenges), as the children's risk assessment requires.

**Design rules: unlocking 18+ content in the UK**
- Pass a **"highly effective" age check** through a specialist provider. Ticking a box doesn't count for Ofcom. Methods include:
  - facial age estimation;
  - a photo-ID match;
  - a credit-card check;
  - open banking;
  - a mobile network check;
  - a digital ID app.

  Providers include Yoti, Persona and Stripe Identity, at roughly a few cents to about a dollar per check.
- Only people who want 18+ content take the check, which keeps the cost down.
- The provider keeps the evidence. **Slop stores only "UK age check passed: yes", the date and the method,** never ID documents or face images.
- **Logged-out UK visitors** see the under-18 view.

**Design rules: US and EU**
- Users confirm their date of birth at sign-up, and Slop records the confirmation. 18+ games are shown only to accounts that say they're 18+, unless a country requires more (§5.7).
- Accounts later found to be under 16 are deleted.

**Sexually explicit content is banned** (decided 2026-10-17, §5.11). So "18+" on Slop means mainly violent or mature games, which the UK under-18 view hides until an age check.

**⚖️ For the lawyer:**
- Does "under-18 view by default, age check to unlock 18+" meet the Online Safety Act's children's duties for a service of Slop's size?
- Which age-check methods should we offer?
- Is self-declared date of birth enough in the US/EU (US state age-check laws, the EU's national rules)?
- Is self-declaration enough for 18+ donations, given that UK teens can have bank cards?

---

## 7. Privacy and data protection

**Position:** Slop collects as little personal data as possible. For the GDPR, Slop is the **controller** of account, vote, donation-history and message data. Stripe is an **independent controller** for payment data.

**Design rules**
- Privacy policy covering:
  - What we collect, why, and the legal basis.
  - Retention periods.
  - Who receives the data: Stripe, the cloud provider, the email provider, the GitHub/GitLab APIs.
  - International transfers, under the EU-US Data Privacy Framework or standard contractual clauses.
  - Users' rights.
- **Public by default:** votes and reviews (decided). Users are told at sign-up and can make them private.
- **Never public:** who donated (decided). A donor's name goes to the maker only if the donor chooses to share it.
- **Recommendations** based on a user's library, Interested list and votes: allowed under legitimate interest, with a simple "don't personalise" switch.
- Data export and deletion are self-service. Votes are anonymized on deletion, not erased (`METRICS.md` §6).
- **Data breaches:** report to the regulator within 72 hours (GDPR Art. 33). Incident plan in `SECURITY.md` §13.
- **Cookies:** only essential cookies on the website without consent (ePrivacy rules; TDDDG in Germany). Any analytics needs consent or must be cookieless.
- Data processing agreements with every service provider.

---

## 8. Accessibility

The **European Accessibility Act** (in force since 28 June 2025) covers online services where consumers make transactions. Very small businesses providing services are exempt (fewer than 10 staff and annual turnover or balance sheet of €2M or less). Design to the WCAG 2.2 AA standard anyway, so that growing out of the exemption doesn't mean starting over. ⚖️ Does Slop qualify as a very small business, and does a donation count as an online transaction?

## 9. Sanctions and export controls
- Accounts from comprehensively sanctioned regions are blocked at sign-up, using a country list the lawyer approves. Stripe screens payments.
- Encryption in open-source software that's publicly available is generally not subject to US export rules (EAR §734.3(b)(3)). ⚖️ Confirm this for Slop's own client and for Private Slop, which isn't public.

## 10. Code signing and responsibility
**Position (from the 2026-10-17 decisions):**
- The **maker signs** each upload.
- **Slop adds its own signature** to the release only after verifying and scanning it.
- That signature means "this is the file the maker uploaded, and it passed our checks". It's **not** a promise that the software is safe.
- Slop **doesn't sign apps with its own operating-system certificate** (Windows Authenticode / Apple Developer ID). That keeps Slop from appearing as the "publisher" of other people's software.

**⚖️ For the lawyer:**
- Does launching apps through the Slop client, rather than through the operating system's own download warnings, make Slop more responsible for them? (Steam works similarly.)
- Wording for the "What scanning means" page.

---

## 11. Legal documents Slop needs

| Document | Who agrees to it | Key points |
|---|---|---|
| **Terms of Service** | Everyone with an account | 16+. UK under-18s get the under-18 view. Wyoming law for US users, with local consumer rights preserved in the EU/UK. Slop is a directory and delivery service. Apps are the makers' responsibility. Scanning limits. Rules of conduct. Limits on liability (as far as the law allows). Account removal. Which law applies. Changes to the terms. |
| **Donation Terms** (inside the Terms of Service) | Donors | Voluntary and nothing in return. Paid to the maker through Stripe. All-in total shown. Final (only the maker can refund). Server donations recur until cancelled, with how to cancel. |
| **Publisher Agreement** | Makers, branch authors | Rights to all code and assets (including AI output). Open-source licence. Compliance with licences and the AI Act. No loot boxes or scams. Donation rules (§3.2). Slop's 5% fee. Taxes are the maker's. Indemnity. Signing keys. Rules for the scans and the kill switch. |
| **Humans Terms + default Service Agreement** | Creatives and clients | Slop is an intermediary, not a party or judge. Direct payment. 5% fee for creatives. The 7-day completion window. Refunds are the creative's. Truthful AI-use disclosure. Who owns the work. Rules for ratings and reviews. Tax reporting (DAC7). |
| **Official Team Terms** | Verified studios | Verified through their GitHub organization (`DESIGN.md` §14). Their responsibilities. Losing verification. |
| **Privacy Policy** + cookie notice | Everyone | §7 |
| **Content & Conduct Policy** | Everyone | Banned content, apps, AI uses and conduct |
| **Copyright & Trademark Policy** | Everyone | DMCA agent, notice and counter-notice, repeat infringers |
| **Illegal content reporting** (notice-and-action form) | Anyone | DSA Art. 16. Statement of reasons. Appeals. |
| **"What scanning means"** | Public | The checks we run, their limits, no guarantee |
| **Vulnerability Disclosure Policy** + `security.txt` | Researchers | Safe harbour for good-faith research |
| **Impressum / legal notice** | Public | If required where the company is based |

## 12. Word list (product, docs, marketing, support)

The words have to match what actually happens (§0). Using the right words supports our position; using the wrong ones weakens it.

| Use | Avoid | Why |
|---|---|---|
| Get · Add to library · Install | Buy · Purchase · Order · Checkout (for apps) | Apps are free. Nothing is sold. |
| Donate · Donation · Server donation · Support the maker | Pay for · Price · Subscription · Membership · Supporter pack · Unlock | Donations give nothing in return (§3.2–3.3) |
| Donor | Customer · Buyer | |
| Maker · Publisher | Seller · Vendor (for apps) | |
| Client (Humans) · Service · Quote · Job | — | Humans **is** a sale, so normal sale words fit here |
| Paid directly to @maker via Stripe | Slop pays out · Your Slop balance · Slop wallet | Slop never holds money (§3.1) |
| Slop's platform fee (5%) | Slop's cut of the sale · commission on sales | Nothing was sold (§3.5) |
| Scanned · Passed Slop's checks · Security report | Safe · Virus-free · Guaranteed · Secure · Trusted | §5.2 |
| % Not Slop, rated by players | "Slop says this is slop" | Votes are users' opinions (§5.5) |

The mockup's wording was updated to match on 2026-10-17: no in-app purchases, no supporter packs, the donor-pays-fees breakdown, "platform fee", and "scanned" instead of "safe".

## 13. Records that support our position

Keep these so we can prove the design works as stated:
1. **Stripe charge records** showing the **maker's account** as merchant and Slop only as the application-fee recipient. This proves Slop never held funds.
2. **Versioned copies** of every donation, server-donation and Humans payment screen, with release dates (screenshots and code tags). This shows what users saw.
3. **Acceptance records:** which version of the Terms, Donation Terms, Publisher Agreement or Humans Terms each user accepted, and when. The same for age confirmation.
4. **Scan records:** each check, its result and the time, kept for the life of the app plus [period to be set]. This shows careful process.
5. **Notice-and-action records:** what was reported, when, the decision, the reason, and how long it took. **Repeat-infringer records.**
6. **Kill-switch records:** what was revoked, when, by whom, and the users notified.
7. **A design check** that no API exists for "has this player donated?". Re-checked at every release (§3.2 rule 2).
8. **Server-donation cancellations:** time from request to cancellation (should be immediate).
9. **Humans:** the agreed Service Agreement, delivery and completion times, and refund actions. No Slop rulings, because there are none.

## 14. Team decisions and remaining questions

**Decided 2026-10-17:**
- **A.** IronPoint Development LLC, US-based.
- **B.** No in-app purchases.
- **C.** One Stripe account per app, and the publisher handles any split.
- **D.** Donors get nothing.
- **E.** Chat is between people, and Slop doesn't read messages.
- **F.** Launch in the **US, EU and UK** (§2, §3.5, §4.3, §5.8–5.10, §8).
- **H.** **End-to-end encrypted** chat (§4.6).
- Slop **doesn't store makers' builds or source code**. Builds live on the makers' release pages, and makers need public code (Private Slop excepted).
- **G.** The LLC is formed in **Wyoming** (§2).
- **Age:** 16+ everywhere. UK accounts start in an **under-18 view** with 18+ content hidden, and a real age check unlocks it. No VPN detection (§6).

**Still open:** nothing for the team right now. The next step is the lawyer review (§15).

## 15. Questions for the lawyer (all ⚖️ items)
1. Money-transmitter / payment-institution status under the direct-charge model (§3.1).
2. Donations outside VAT/sales tax. Using the word "donation" in countries where it suggests charity (§3.2).
3. Automatic-renewal and online-cancellation rules for server donations (§3.3).
4. Whether the donor-paid fee counts as a surcharge (§3.4).
5. VAT/sales tax on Slop's 5% fee, and tax registrations (§3.5).
6. Stripe Connect responsibilities: KYC, losses, 1099-K (§3.7).
7. Whether "Slop doesn't settle disputes" holds for consumer clients. Required dispute-resolution notices (§4.1).
8. DSA status and the small-enterprise exemptions (§4.2).
9. DAC7 reporting for Humans, and for donations (§4.3).
10. Employment status under the Platform Work Directive as each country adopts it (§4.4).
11. Defamation and GDPR for ratings of creatives (§4.5).
12. Confidentiality rules for chat and what scanning is allowed (§4.6).
13. Wording and limits of liability for scanning (§5.2, §10).
14. Duties for reporting illegal content, and AI Act duties passed to makers (§5.3).
15. GPL obligations and AI-copyright terms (§5.4).
16. Trademark clearance for "Slop" (§5.6).
17. Age ratings, IARC and German youth protection (§5.7).
18. Age checks: is self-declaration enough at 16+ in the US/EU and 18+ for payments? Does the UK under-18 view with age checks to unlock 18+ content meet Ofcom's children's codes (§6, §5.9)? Is the sexual-content ban (§5.11) enough to keep Slop out of the pornography age-check rules?
19. GDPR set-up: EU representative, transfers, legitimate interest for recommendations (§7).
20. European Accessibility Act exemption (§8).
21. Sanctions country list and export controls (§9).
22. Responsibility for apps launched through Slop instead of the OS's own checks (§10).
23. LLC set-up, insurance, DBA for "Slop", and the EU/UK representatives if serving those regions (§2).
24. Whether maker-hosted builds make Slop a directory/location tool rather than a host (§5.1).
25. US state sales tax on Slop's 5% platform fee: some states tax digital or SaaS services (§3.5).
26. The Stored Communications Act and a law-enforcement request policy for chat (§4.6).
27. VAT: donations outside scope, the fee as an electronically supplied service, non-Union OSS and UK registration (§3.5).
28. UK Reporting Rules for Digital Platforms for Humans (§4.3).
29. EU AI Act: which duties pass to makers, the ban on high-risk apps, and the status of the Digital Omnibus delays (§5.8).
30. Cyber Resilience Act and Product Liability Directive: are donation-funded open-source makers "commercial"? What are Slop's duties as a distributor (§5.8)?
31. UK Online Safety Act: risk assessments, children's duties for 16–17-year-olds, age assurance for 18+ content, and encrypted chat (§5.9).
32. Review-authenticity statements and UK drip-pricing rules (§5.10).
33. Sexual content (§5.11):
    - the wording of the ban (where explicit ends);
    - TAKE IT DOWN Act compliance;
    - the ban on depictions of minors.
34. US **app-store age-verification laws** (Utah, Texas, Louisiana, passed in 2025, some challenged in court): do they apply to Slop as an app store, and what's their current status?
