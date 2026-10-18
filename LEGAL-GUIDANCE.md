# Slop — Legal Guidance for Development

**Status:** Working guidance for the team · **This is not legal advice.**
**Last updated:** 2026-10-17 (laws checked against public sources on this date) · **Company:** IronPoint Development LLC (Wyoming, US)
**Related:** [`LEGAL.md`](LEGAL.md) (our legal positions, under review by a lawyer) · [`DESIGN.md`](DESIGN.md) · [`PAYMENTS.md`](PAYMENTS.md) · [`SECURITY.md`](SECURITY.md) · [`ARCHITECTURE.md`](ARCHITECTURE.md) · [`METRICS.md`](METRICS.md)

---

## 0. What this document is for

`LEGAL.md` sets out **what Slop claims to be** (a directory, not a seller; donations, not purchases) and the questions a lawyer has to answer. That review comes before launch.

**This document is for the time in between.** It turns the law that applies to Slop into **rules for people building it**, so the code we write now doesn't have to be rewritten after the review. It's written from public sources by the team, not by a lawyer. Where the lawyer's answers differ, **their answers win** and this document gets updated.

### How to use it
- Every rule has an ID (for example **PAY-3**). Put it in pull requests and code comments: `// PAY-3: show the all-in total`.
- Each section says how settled the law is:
  - **[Settled]**: the law clearly says this. Follow it.
  - **[Our reading]**: our best reading of the law. Build it this way; the lawyer is confirming it.
  - **[In flux]**: the law is changing or being fought in court. Build it so that it can be **switched per country**, without a rewrite.
- **Build switches, not forks.** Anything that differs by country (age rules, cookie consent, fee display, which features are on) is **configuration keyed by country** (and by US state where needed). It is never hard-coded `if (country == "GB")`. Several rules below depend on this.
- If a rule here conflicts with `LEGAL.md`, follow the **stricter** rule and raise it.
- §2 lists the changes that **must not be built** until the lawyer has seen them.

---

## 1. The twelve rules that matter most

1. **Slop never touches anyone else's money.** All payments are direct charges into the maker's or creative's own Stripe account. (§3.1)
2. **Donations give nothing back, and apps can't tell who donated.** (§3.2)
3. **Show the full amount the donor pays from the start,** never added at the last step. (§3.3)
4. **Every app clearly names its maker,** and Slop can reach that maker. (§6.2)
5. **Never change a maker's code or builds.** Slop builds from their source and signs it, but it doesn't add to it, repackage it or alter it. (§6.1, §5.1)
6. **Say only what is true:** "scanned", never "safe". (§5.5, word list in `LEGAL.md` §12)
7. **Act on reports quickly and record everything:** what was reported, the decision, the reason and the time taken. (§5, §15)
8. **Banned content stays banned:** sexual content, child abuse material, nudify apps, gambling, malware, banned AI uses. (§5.4)
9. **Accounts are 16+. Under-18s get safe defaults** wherever the law requires them, controlled per country. (§9)
10. **Collect as little data as possible, and ask first where the law requires it.** (§8)
11. **Slop's servers never hold chat keys or message content.** (§10)
12. **The Slop client is our own product.** We must keep it secure, fix it, and report actively exploited vulnerabilities in it. (§6.1)

---

## 2. Stop: check with the lawyer before building any of these

These changes would undo a legal position that the rest of the design depends on.

| If someone proposes… | Why it's a problem | Law involved |
|---|---|---|
| Holding money: wallets, balances, credits, escrow, payouts from Slop's account, or splitting payments between makers | Slop would receive and pass on other people's money, which could require money-transmitter / payment-institution licences | 31 CFR 1010.100(ff); state money-transmission laws; PSD2 (EU); PSRs 2017 (UK) |
| Anything a donor gets in return: badges, supporter lists, perks, early access, a "has donated" API | Donations would become sales, which brings VAT and sales tax, consumer-sale rights, and possibly the CRA/PLD "commercial activity" test | VAT Directive Art. 2 and *Tolsma* (C-16/93); CRA recitals; PLD |
| In-app purchases or paid apps | Slop would be selling digital goods: marketplace VAT rules (deemed supplier), consumer digital-content law | Implementing Reg. 282/2011 Art. 9a; Directive 2019/770 |
| Loot boxes, gambling or prize mechanics | Gambling law | National gambling laws (e.g. Belgium) |
| Adult or sexual content in any form | Strict age-verification laws, and Stripe's restricted-business list | UK OSA; *Free Speech Coalition v. Paxton* (2025); Stripe terms |
| Slop modifying, wrapping or re-signing makers' builds as their publisher (e.g. Slop's own Authenticode / Apple Developer ID certificate) | Slop could become the "manufacturer" or "publisher" of other people's software | CRA Arts. 21–22; PLD Art. 8 |
| Storing makers' builds or source on Slop servers | Moves Slop from "directory that links" to "host"; cost and liability | 17 U.S.C. §512(c) vs (d); DSA Art. 6 |
| Scanning, logging or keeping chat content on the server; server-held chat keys | Breaks the E2E position and confidentiality-of-communications law | ePrivacy Directive Art. 5(1); 18 U.S.C. §2701 ff. |
| Slop choosing creatives' prices, assigning work, setting response times or deactivating creatives automatically | Creatives could be presumed to be employees | Platform Work Directive 2024/2831; US worker-classification tests |
| Installing apps **onto phones** (Android "Send to phone") for users in Utah, Texas, Louisiana, or California (from 2027) | Slop becomes a mobile "app store" under the state app-store age laws | §9.4 |
| Using makers' code, users' content or chat to train AI models | Copyright, licence, GDPR purpose limits | DSM Directive 2019/790 Art. 4; GDPR Art. 5–6 |
| Advertising, paid ranking or sponsored placements | Labelling and ad-transparency duties; profiling rules for minors | UCPD Annex I pt 11a; DSA Arts. 26 and 28; ePrivacy |
| Expanding to new countries (Australia, Canada, Brazil, India…) | Each country has its own online-safety, tax and privacy laws | — |
| Launching on iOS | Apple's rules, and the EU Digital Markets Act alternative-marketplace terms | DMA (EU) 2022/1925 |

---

## 3. Money and payments

### 3.1 Staying out of money transmission — [Our reading]
**The law.**
- **US federal:** a "money transmitter" is someone who **accepts** funds from one person and **transmits** them to another (31 CFR 1010.100(ff)(5)).
- **US states:** each state has its own licence law, and many now follow the CSBS Money Transmission Modernization Act.
- **EU:** PSD2 (Directive (EU) 2015/2366, being replaced by PSD3 and the Payment Services Regulation) licenses anyone who carries out "payment services".
- **UK:** the Payment Services Regulations 2017 (SI 2017/752) do the same.

**What it means for Slop.** Slop's main argument isn't an exemption. It's that **Slop never accepts the funds at all**:
- Stripe charges the donor on the maker's own Stripe account.
- Stripe takes Slop's application fee out and pays it to Slop.

That only stays true while the code keeps it true.

**Rules**
- **PAY-1:** create every charge as a **direct charge on the connected account**, with `application_fee_amount`. Never use destination charges, "separate charges and transfers", or the `transfer_data` parameter.
- **PAY-2:** Slop's own Stripe balance only ever contains Slop's fees. A daily reconciliation job checks this (`PAYMENTS.md` §8). Any other money arriving in it is an incident.
- **PAY-3:** Slop doesn't issue refunds itself. The refund button calls Stripe **on the maker's account**, as the maker, after the maker clicks it.
- **PAY-4:** only Stripe Connect **Standard** accounts (the maker has their own dashboard and Stripe holds them liable for losses). Changing the account type needs the lawyer (§2).

### 3.2 Keeping donations as gifts — [Our reading]
**The law.**
- **VAT:** VAT only applies to supplies made "for consideration" (VAT Directive 2006/112/EC Art. 2). The EU Court has held that voluntary payments to a street musician aren't consideration, because nothing is given in exchange (*Tolsma*, C-16/93, 1994).
- **Product laws:** the Cyber Resilience Act and the new Product Liability Directive both exclude free open-source software that isn't supplied as a commercial activity. Their recitals treat donations made without a profit motive as non-commercial.

So the whole donation model depends on there being **no link between paying and getting something**.

**Rules**
- **PAY-5:** the API has **no endpoint, field, webhook, SDK call or token** that shows whether a user donated. This includes Slop's own client UI: nothing in another user's or the maker's app view changes because of a donation. Add an automated test that fails if a donation field ever appears in an app-facing API schema.
- **PAY-6:** donor names and messages go only to the maker, only if the donor opts in, and are never shown publicly.
- **PAY-7:** use the words in `LEGAL.md` §12 in UI text, emails, receipts and Stripe metadata. Stripe `description` and `statement_descriptor` fields must say "donation", not "purchase" or "order".
- **PAY-8:** never put VAT or sales tax **on** a donation, and never take it **out** of one. Tax only ever applies to Slop's fee (§3.5).
- **PAY-9:** the maker sees "Donations may be taxable income for you. Slop doesn't give tax advice." before enabling donations, and the acceptance is recorded.

### 3.3 Showing what the donor pays — [Our reading; one part In flux]
**The law.**
- **UK:** the Digital Markets, Competition and Consumers Act 2024 (DMCCA), Part 4 Chapter 1, in force since 6 April 2025, makes **drip pricing** an unfair practice. Unavoidable fees must be part of the first price shown.
- **US:** California's "honest pricing" law (Civil Code §1770(a)(29), since 1 July 2024) has the same effect. Several other states have similar rules.
- **Surcharges:**
  - **EU:** extra charges for paying with consumer cards are banned (PSD2 Art. 62(4)).
  - **UK:** the same ban (Consumer Rights (Payment Surcharges) Regulations 2012, SI 2012/3110).
  - **US:** card-network rules limit credit-card surcharges and ban debit-card surcharges.
- **Our position** (`LEGAL.md` §3.4) is that the donor-pays-fees amount is **one all-in price, the same for every payment method**, not a surcharge. A lawyer hasn't confirmed this yet.

**Rules**
- **PAY-10:** wherever an amount is offered, the **largest, first number is what the donor pays**. Preset buttons show the total ("**$3.40** · @maker gets $2.85"), not only the amount the maker receives. This applies to the donate box, server donations and Humans quotes.
- **PAY-11:** the processing addition is calculated from **one published rate per currency**, the same for cards, wallets and bank payments. Never vary it by payment method or card type.
- **PAY-12:** **build the fallback now.** A per-country setting `donor_covers_fees` (on/off). When it's off, the donor's chosen amount is the total, and the fees come out of what the maker receives. If the lawyer says this counts as a surcharge in the EU/UK, we switch it off there with no code change.
- **PAY-13:** never pre-tick any box that adds money (Consumer Rights Directive 2011/83/EU Art. 22; UK CCR 2013 reg. 40).

### 3.4 Server donations (recurring) — [Our reading; UK In flux]
**The law.**
- **US federal:** ROSCA (15 U.S.C. §§8401–8405) requires clear terms, express consent and a simple way to cancel recurring online charges. The FTC's wider "click-to-cancel" rule was **struck down** by the Eighth Circuit on 8 July 2025, so ROSCA and state laws are what apply.
- **California:** the Automatic Renewal Law (Bus. & Prof. Code §§17600–17606, amended from 1 July 2025) requires:
  - consent to the renewal terms specifically;
  - online cancellation;
  - reminders;
  - an acknowledgement email.
- **Germany:** a cancellation button is required for online contracts (§312k BGB).
- **UK:** the DMCCA's subscription-contracts regime hasn't started yet. Recent reports say January 2027; earlier government statements said spring 2027.
- These laws are written for **contracts**. A gift with nothing in return may fall outside them, but **we build as if they apply**.

**Rules**
- **PAY-14:** before confirming, show the amount, "every month until you cancel", the first charge date and how to cancel, with a separate unticked confirmation. Save a copy of exactly what was shown (§15).
- **PAY-15:** cancelling takes effect immediately, works online in two clicks with no reason needed, and sends a confirmation email.
- **PAY-16:** email a reminder before the first renewal, and once a year after that.
- **PAY-17:** if the maker marks the servers as shut down, cancel every server donation for that app automatically and email the donors.

### 3.5 Tax — [Settled for the rules; amounts need an accountant]

| Tax | What applies | Rule |
|---|---|---|
| **EU VAT on Slop's fee** | Slop's fee is an "electronically supplied service" (Implementing Reg. 282/2011 Art. 7 and Annex I). For makers who aren't VAT-registered businesses, it's taxed where the maker lives (VAT Directive Art. 58). A non-EU business can register once under the **non-Union OSS scheme** (Arts. 358a–369), with no minimum threshold. For VAT-registered businesses, the maker accounts for it under the reverse charge (Art. 196). | **TAX-1:** at onboarding, ask for a VAT number and check it in VIES. Without a valid number, treat the maker as a consumer (Implementing Reg. Art. 18). **TAX-2:** store **two pieces of evidence that agree** on the maker's country, e.g. Stripe billing country and IP country (Implementing Reg. Arts. 24b and 24f). |
| **UK VAT on Slop's fee** | A non-UK business supplying digital services to UK consumers must register from the first sale. There's no threshold (VATA 1994 Sch. 1A). | Same as TAX-1/TAX-2, but check UK VAT numbers with HMRC's checker. |
| **US sales tax on Slop's fee** | Some states tax SaaS or digital services. After *South Dakota v. Wayfair* (2018), a state can require out-of-state sellers to collect tax once they pass its sales threshold. | **TAX-3:** record each maker's state. Calculating the tax is the accountant's job (Stripe Tax can do it). |
| **Fee is VAT-inclusive** | Decided: the 5% includes any VAT, and Slop pays it out of the 5%. | **TAX-4:** fee statements show the net fee, the VAT and the rate. The headline stays "5%". |
| **Donations** | Outside VAT (§3.2). | PAY-8. |
| **Humans: EU DAC7** | Platforms must report sellers of personal services (Directive (EU) 2021/514). There's no minimum for services. A non-EU platform registers in one EU country. Reports are due by 31 January. | **TAX-5:** before a creative with any EU link is paid, collect their name, address, tax ID (TIN), date of birth (individuals) or business registration number, and the country that issued the TIN. Block payouts if this is missing at year end. |
| **Humans: UK reporting rules** | The UK's equivalent of DAC7 (SI 2023/817), in force since 1 January 2024. Reports go to HMRC by 31 January. | Same data as TAX-5, for creatives with UK links. |
| **US 1099-K** | Stripe files these for Standard accounts. The threshold went back to **$20,000 and 200 transactions** under Pub. L. 119-21 (2025). | Nothing for Slop to build. Confirm with Stripe. |

### 3.6 Stripe's own rules — [Settled]
Stripe's Services Agreement and its **restricted businesses list** bind every maker and creative. If Stripe closes accounts over content, donations stop for **everyone**.
- **PAY-18:** Slop's Content Policy must be **at least as strict** as Stripe's restricted list for anything that can receive donations or Humans payments. Re-check the list every quarter.

### 3.7 Sanctions — [Settled]
**The law.**
- **US:** OFAC rules (31 CFR Chapter V). Comprehensive embargoes currently cover **Cuba, Iran, North Korea, and the Crimea, so-called "DNR" and "LNR" regions of Ukraine**. The **Syria** programme was largely lifted in 2025 (EO 14312), but some restrictions remain.
- **EU and UK:** their own regimes, including wide restrictions on Russia and Belarus.

**Rules**
- **PAY-19:** the list of blocked sign-up regions is **configuration approved by the lawyer**, never hard-coded. Re-check it every quarter.
- **PAY-20:** Stripe screens payments. Slop blocks sign-up and payments from the blocked regions, and screens the names of makers, creatives and official teams if it ever pays anyone itself (it doesn't today).

---

## 4. Humans (the one real sale)

### 4.1 Consumer law when a creative is a business — [Settled in the EU/UK]
**The law.**
- **The client may be a consumer.** A client hiring a creative can be an individual, not a business. If the creative is a **trader** (in business), EU and UK consumer law applies to the deal.
- **Marketplaces must say whether the seller is a trader**, and what that means for the consumer's rights (CRD Art. 6a, added by the Omnibus Directive 2019/2161).
- **14-day cancellation right:** consumers can withdraw from distance contracts for services within 14 days (CRD Art. 9; UK CCR 2013 regs 29–38). If work starts within those 14 days, the consumer must **expressly ask** for it. They lose the right once the service is **fully performed**, but only if they **acknowledged** that this would happen (CRD Art. 16(a)).
- **New EU "withdrawal button":** from **19 June 2026**, contracts made through an online interface must offer a **withdrawal function** on that interface (CRD Art. 11a, added by Directive (EU) 2023/2673). That interface is Slop's Humans checkout.
- **No ODR link needed:** the EU ODR platform closed on 20 July 2025 (Regulation (EU) 2024/3228). Germany's VSBG §36 notice applies only to traders with more than 10 employees.

**Rules**
- **HUM-1:** during creative onboarding, ask: "Are you offering services as a business (trader)?" Show the answer on the creative's profile and on every quote, with a fixed sentence explaining what it means for the client's rights.
- **HUM-2:** when the creative is a trader and the client is a consumer in the EU/UK:
  - the quote includes the **14-day withdrawal information** and the model withdrawal form (CRD Annex I);
  - if work starts earlier, the client ticks an **unticked** box: "Start now. I understand I lose my right to cancel once the work is fully done."
- **HUM-3:** build a **"Withdraw from this contract" button** on the job page for EU consumers while the withdrawal period is open. It notifies the creative and records the time.
- **HUM-4:** the 7-day auto-completion (`PAYMENTS.md` §7) **never shortens** a legal withdrawal period. Completion and withdrawal are tracked separately.
- **HUM-5:** "Slop doesn't settle disputes" is fine as a statement about Slop's role. It **must not** suggest the client has no rights. Always add: "Your rights are against @creative."

### 4.2 Platform-to-business rules — [Our reading]
**The law.** The EU P2B Regulation (EU) 2019/1150 protects businesses that sell to EU consumers through a platform. That covers creatives who are traders, and probably makers who are businesses. It requires:
- terms that are easy to find;
- **15 days' notice** of changes to the terms (Art. 3);
- a **statement of reasons** when Slop restricts or suspends a business (Art. 4), and **30 days' notice** before ending the relationship, except in serious cases;
- an explanation of the **main ranking factors** (Art. 5).

The internal complaint-handling duty (Art. 11) doesn't apply to small enterprises.

**Rules**
- **HUM-6:** Terms of Service / Publisher Agreement / Humans Terms changes are versioned, emailed with **at least 15 days' notice**, and accepted on the next sign-in. Each user's accepted version is stored (§15).
- **HUM-7:** every restriction, delisting or suspension produces a written reason in the user's dashboard (this also covers DSA Art. 17, §5.2).
- **HUM-8:** publish a "How ranking works" page listing the main factors (votes, recency, Pulse, Interested, personalisation). Update it when the ranking code changes.

### 4.3 Creatives are independent — [In flux in the EU]
**The law.** The EU Platform Work Directive 2024/2831 must be in national law by **2 December 2026**. It presumes employment where a platform **directs and controls** work. US tests are similar (for example, control over pay, hours and how the work is done).

**Rules**
- **HUM-9:** Slop never sets or limits prices, assigns jobs, requires response times, sets schedules, monitors how work is done, or automatically restricts creatives based on acceptance rates or ratings.
- **HUM-10:** if search ranking ever uses creatives' performance data, that needs the lawyer first (§2).

### 4.4 Ratings of people — [Our reading]
**The law.**
- **GDPR:** ratings of identifiable people are personal data. The basis is legitimate interest (Art. 6(1)(f)), with a right to object (Art. 21).
- **Defamation:** in the US, Section 230 protects Slop for users' reviews. In the UK, website operators have a defence under the Defamation Act 2013 s.5 if they follow the notice process.
- **EU:** the DSA hosting rules apply.

**Rules**
- **HUM-11:** only a client with a completed job can rate. The creative can reply publicly and report the review.
- **HUM-12:** reported reviews are checked against the rules (insults, private information, false statements of fact), **not** against whether they're negative (§13).

---

## 5. Content, apps and notices

### 5.1 United States — [Settled]
- **Section 230** (47 U.S.C. §230)
  - **What it covers:** Slop is protected for users' content (apps, reviews, profiles, messages) and for good-faith moderation.
  - **What it doesn't cover:**
    - **Slop's own words:** badges, "scanned", our descriptions and our emails;
    - intellectual property;
    - federal crimes;
    - sex-trafficking content (FOSTA);
    - claims about how **our own product is designed** (*Lemmon v. Snap*, 9th Cir. 2021).

    Slop's wording, client, ranking and chat design are therefore our own responsibility.
  - **Rule CON-1:** never edit, add to or "improve" a user's content in a way that changes its meaning. Fixing formatting is fine.
- **DMCA** (17 U.S.C. §512)
  - **CON-2:** register a **designated agent** with the US Copyright Office before launch. It's an online filing that must be **renewed every 3 years** (37 CFR 201.38). Publish the agent's details on the Copyright Policy page.
  - **CON-3:** build the takedown form to collect every item §512(c)(3) requires. Handle counter-notices under §512(g): restore the content **10–14 business days** after a valid counter-notice unless the complainant tells us they've gone to court.
  - **CON-4:** apply a written **repeat-infringer policy** (§512(i)). The system counts strikes per account and ends accounts at the threshold.
- **TAKE IT DOWN Act** (Pub. L. 119-12). Removal duties in force since **19 May 2026**, enforced by the FTC, with penalties of about $53,000 per violation.
  - **CON-5:** offer a request form for **non-consensual intimate images**, including AI-generated ones. Remove the image and known identical copies **within 48 hours** of a valid request. Treat these requests as top priority so the 48 hours can be met.
- **Child sexual abuse material** (18 U.S.C. §2258A)
  - **CON-6:** report to NCMEC's CyberTipline as soon as reasonably possible after learning of it.
  - **Preserve** the report contents for **1 year**, a period set by the REPORT Act (2024).
  - Access to preserved material is limited to a small named group and logged.
  - Never delete it as part of an ordinary content removal.

### 5.2 European Union — [Settled]
- **Digital Services Act** (Regulation (EU) 2022/2065).
  - While Slop is a micro or small enterprise, it's exempt from most extra platform duties (Arts. 19 and 29). **The hosting basics always apply.**

  | DSA duty | Rule |
  |---|---|
  | Contact points for regulators and users (Arts. 11–12) | **CON-7:** published contact page, with a monitored email address and form |
  | EU legal representative (Art. 13) | Appointed before EU launch, and named on the contact page |
  | Clear terms, including how moderation works (Art. 14) | Content Policy and Terms describe moderation, automated tools and appeals |
  | Notice-and-action (Art. 16) | **CON-8:** a report form on every app, review, profile, listing and message report, with fields for the reason, location, the reporter's name and email (optional for CSAM reports), and a good-faith statement. Acknowledge receipt. |
  | Statement of reasons (Art. 17) | **CON-9:** every removal, demotion, delisting, suspension or demonetisation sends the user: what was restricted, the facts, the rule or law, whether automation was used, and how to appeal |
  | Report threats to life or safety (Art. 18) | **CON-10:** a moderator path to notify police in the relevant EU country |
  | Complaints and appeals (Art. 20) | Small enterprises are exempt, but **build it anyway** (GDPR Art. 22 and UK OSA need it too) |
  | Dark patterns (Art. 25) | Small enterprises are exempt. Follow it anyway: no tricks that push users into choices |

- **Terrorist content** (Regulation (EU) 2021/784)
  - **CON-11:** a removal order must be acted on **within 1 hour**. That needs:
    - an on-call contact point (Art. 15);
    - a legal representative in the EU (Art. 17);
    - an admin tool that removes or geo-blocks content in one step.
- **E-evidence** (Regulation (EU) 2023/1543 and Directive (EU) 2023/1544, applying since **18 August 2026**)
  - **What it requires:** chat and hosting providers that serve EU users must name an **EU addressee** (an establishment or legal representative). Production orders must be answered **within 10 days**, or **8 hours in emergencies**.
  - **CON-12:** appoint the addressee before EU launch; DSA representative firms often offer this too. Add these orders to the law-enforcement request process (§10).

### 5.3 United Kingdom: Online Safety Act 2023 — [Settled; details depend on size]
Slop is a user-to-user service with UK users. Ofcom enforces the Act.

| Duty | Since | Rule |
|---|---|---|
| **Illegal content risk assessment** and safety measures (ss.9–10, Ofcom Illegal Content Codes) | 17 Mar 2025 | **CON-13:** write the risk assessment **before UK launch**, using Ofcom's risk profiles. Review it every year, and **before any significant feature change** (chat, Humans, new content types). Name one accountable person. |
| **Children's access assessment** and **children's risk assessment** (Part 3 ch. 4, ss.11–12, Protection of Children Codes) | 25 Jul 2025 | Under-18 UK users are allowed, so both are needed. The design response is the UK under-18 view (§9). |
| **Reporting and complaints** (ss.20–21) | 2025 | CON-8 and the appeals path cover this |
| **Record keeping** (s.23) | 2025 | §15 |
| **Reporting CSEA to the National Crime Agency** (s.66; SI 2026/268) | **7 Apr 2026** | **CON-14:** register with the NCA, and report detected child sexual exploitation and abuse content to the NCA in the required format. This is **in addition to** NCMEC (CON-6). Build the reporting tool so one moderator action files both. |

### 5.4 Banned content and apps (one list for the Content Policy and the scanners)

| Banned | Why (main law) | Since |
|---|---|---|
| Child sexual abuse material, including drawn or AI-generated | 18 U.S.C. §§2252A, 1466A; UK Coroners and Justice Act 2009 s.62; EU Directive 2011/93/EU | Always |
| Sexually explicit content (decided) | Stripe terms; age-verification laws (`LEGAL.md` §5.11) | Decided 2026-10-17 |
| Non-consensual intimate images, and **"nudify" apps** that make sexual images of real people without consent | TAKE IT DOWN Act; UK sexual-deepfake offences; **EU AI Act Art. 5 as amended by the Digital Omnibus on AI** (Regulation (EU) 2026/1744): **new bans on nudify apps and AI that generates child abuse material** | US 19 May 2026 · EU **2 Dec 2026** |
| AI practices banned by the AI Act (manipulation, exploiting vulnerabilities, social scoring, emotion recognition at work or school, face scraping, biometric categorisation of sensitive traits, and others) | AI Act Art. 5; fines up to €35M or 7% of turnover | 2 Feb 2025 |
| High-risk AI (hiring, credit scoring, exams, biometric ID, and others) | Banned by Slop's own rule, to avoid distributor duties (AI Act Art. 24) | Slop rule |
| Malware, hidden crypto-mining, data theft, deceptive data collection | US Computer Fraud and Abuse Act; FTC Act §5; GDPR | Always |
| Gambling, loot boxes, paid random rewards | National gambling laws | Slop rule |
| In-app purchases, or anything sold | Slop rule (§2) | Slop rule |
| Terrorist content | Regulation (EU) 2021/784; UK Terrorism Acts | Always |
| Copyright or trademark infringement | 17 U.S.C. §512; national laws | Always |

- **CON-15:** every banned category maps to a **scan check or report reason**, and to an enforcement action written in the Content Policy.
- **CON-16:** automated rejections always offer **human review** on request (GDPR Art. 22; DSA Art. 20).

### 5.5 What we say about scanning — [Settled]
**The law.** Exaggerated safety claims are deceptive or unfair practices:
- **US:** FTC Act §5 (15 U.S.C. §45).
- **EU:** Unfair Commercial Practices Directive 2005/29/EC.
- **UK:** DMCCA Part 4.

The FTC has repeatedly brought cases over security claims that weren't true.

**Rules**
- **CON-17:** UI text says "Scanned", "Passed Slop's checks" and "See the security report". **Never** "safe", "secure", "virus-free", "trusted", "verified safe" or "guaranteed". Add a lint rule to the build that fails on these words in UI strings.
- **CON-18:** every store page links to "What scanning means", which lists what we check and its limits.
- **CON-19:** the scan pipeline keeps a record of **every check, version and result** (§15). If something gets through, that record is what shows we took reasonable care.

---

## 6. Software and product law

### 6.1 The Slop client is our product — [Settled]
**The law.**
- **Cyber Resilience Act (CRA)** (Regulation (EU) 2024/2847). It covers products with digital elements sold or supplied in the EU as part of a commercial activity. **Slop's client earns fees, so it's commercial and the CRA applies to it.**
  - **Vulnerability and incident reporting** (Art. 14) has applied since **11 September 2026**. It covers actively exploited vulnerabilities and severe incidents:
    - an early warning to the national cybersecurity agency (CSIRT) and ENISA within **24 hours**;
    - a notification within **72 hours**;
    - a final report within **14 days** of a fix (vulnerabilities) or **1 month** (incidents).
  - **Everything else** applies from **11 December 2027**:
    - the security requirements in Annex I;
    - a software bill of materials (SBOM);
    - vulnerability handling;
    - security updates for the **support period, normally at least 5 years** (Art. 13(8));
    - technical documentation;
    - CE marking.

    The Slop client isn't on the "important" or "critical" product lists (Annexes III–IV), so **self-assessment** (Module A) applies.
- **Product Liability Directive (PLD)** (Directive (EU) 2024/2853). From **9 December 2026**, **software is a "product"** for injury, property damage and **destroyed or corrupted data**.
  - Liability is strict: it doesn't need negligence.
  - The Terms can't exclude it.
  - It covers the Slop client, the installer and the updater.
- **Digital Content Directive** (Directive (EU) 2019/770) — [Our reading]. It also covers free digital services where the user "pays" with personal data that isn't strictly needed to run the service (Art. 3(1)). Slop uses data for recommendations, so its own app may be covered. That means legal duties to keep it working and **provide security updates** (Art. 8).
- **US.** Reasonable security is enforced under FTC Act §5. Negligence claims are possible, and Section 230 doesn't cover our own code.

**Rules**
- **SAFE-1:** before launch, set up the CRA reporting process: who decides, the 24/72-hour clock, ENISA single-reporting-platform access, and a template. Include it in the incident plan (`SECURITY.md` §13).
- **SAFE-2:** generate an **SBOM** for every client release (CycloneDX or SPDX) and keep it.
- **SAFE-3:** publish a **support period** for client versions (at least 5 years for security fixes, normally done by auto-update) and a vulnerability disclosure policy with `security.txt`.
- **SAFE-4:** the client installer, updater and uninstaller **never delete or overwrite files outside Slop's library folders**. Test this. Destroyed data is now a product-liability harm in the EU.
- **SAFE-5:** security by default: auto-updates on, least privilege, no admin rights for normal installs (already planned in `SECURITY.md` §7).

### 6.2 Makers' apps: keep the maker as the "manufacturer" — [Our reading]
**The law.**
- **CRA:** someone who **substantially modifies** a product, or puts it on the market **under their own name**, becomes its manufacturer (Arts. 21–22).
- **PLD:** an online platform can be liable like a distributor if it presents a product as its own, or if it **can't identify the manufacturer within one month** when a harmed person asks (Art. 8).
- **Exclusions:** both laws leave out free open-source software not supplied as a commercial activity. For apps made by makers who earn donations, the line hasn't been tested yet (`LEGAL.md` §15, Q30).

**Rules**
- **SAFE-6:** Slop never changes a maker's build. Specifically, Slop does none of these:
  - injecting code;
  - repackaging;
  - wrapping it in Slop-branded installers that change its behaviour;
  - re-signing it with a Slop OS certificate.

  Slop Build only runs the maker's own build in the maker's own repo. The Slop signature is a **distribution attestation**, and its text says so.
- **SAFE-7:** every store page shows "**Made by @maker**", their repo link, and the licence. Slop's branding never appears as the app's publisher.
- **SAFE-8:** keep **verified contact details** for every maker who publishes publicly: their GitHub identity and a verified email, plus Stripe identity if they take donations. Then Slop can name them within one month if asked (PLD Art. 8).
- **SAFE-9:** the **kill switch and revocation** (`SECURITY.md` §5.5) are how Slop responds to dangerous apps. Record every use (§15).

---

## 7. AI law

### 7.1 EU AI Act, as amended in 2026 — [Settled dates; check before relying on them]
The **Digital Omnibus on AI** (Regulation (EU) 2026/1744, in force 27 July 2026) changed several dates. Current timeline:

| Rules | Apply from | Effect on Slop |
|---|---|---|
| Banned practices (Art. 5) | 2 Feb 2025 | Banned on Slop (§5.4) |
| AI literacy (Art. 4), now softened to "**take measures to support**" staff AI literacy | 2 Feb 2025 | Short internal guidance for staff who use AI tools in moderation or scanning |
| General-purpose AI models | 2 Aug 2025 | Not Slop |
| **Transparency** (Art. 50): tell people they're talking to an AI; machine-readable marking of AI-generated audio, images, video and text; label deepfakes. **Not delayed.** (Reports say systems already on the market before 2 Aug 2026 have until 2 Dec 2026 for the marking duty.) | **2 Aug 2026** | Makers' apps with these features. Slop's own features, if any talk to users or generate media. |
| **New Art. 5 bans:** nudify apps, and AI that generates child sexual abuse material | **2 Dec 2026** | Banned on Slop (§5.4) |
| High-risk systems in Annex III (standalone uses) | **2 Dec 2027** (was Aug 2026) | Banned on Slop |
| High-risk AI in regulated products (Annex I) | **2 Aug 2028** | Not relevant |

**Rules**
- **AI-1:** the publish flow asks: "Does this app contain AI features (chatbot, image/audio/video generation, voice cloning, emotion recognition, biometrics)?" If yes, it asks which ones, and requires the maker to confirm the matching Art. 50 duties. The answers appear on the store page as "AI features".
- **AI-2:** the publish flow keeps a short **banned AI uses** checklist (Art. 5 plus high-risk), so makers confirm it in their own words.
- **AI-3:** if Slop adds its own AI features (a support chatbot, AI-written summaries, AI moderation), then:
  - a chatbot says it's an AI (Art. 50(1));
  - generated media is marked (Art. 50(2));
  - AI moderation decisions keep human review (CON-16).

### 7.2 UK and US — [In flux]
- **UK:** there's no AI Act. Existing law applies: data protection (ICO), consumer law (CMA), online safety (Ofcom). Nothing extra to build today.
- **US:** there's no federal AI statute. State laws mostly cover AI used in **consequential decisions** (for example the Colorado AI Act, SB 24-205, whose start date was pushed back; check the current date). Slop's ban on high-risk apps, and the fact that Slop doesn't use AI for such decisions, keeps it outside these laws.
- **AI-4:** don't use AI to make decisions about creatives' or makers' eligibility, pay or account status without a human decision (this also covers HUM-9 and GDPR Art. 22).

### 7.3 Copyright in AI-made work — [Settled in the US; differs in the UK]
- **US:** material generated purely by AI can't be copyrighted. Only human contributions count (US Copyright Office, *Copyright and Artificial Intelligence* Part 2, January 2025; *Thaler v. Perlmutter*, D.C. Cir. 2025).
- **UK:** "computer-generated works" are protected under CDPA 1988 s.9(3). The government has consulted on changing this.
- **EU:** protection requires the author's own intellectual creation (*Infopaq*, C-5/08). That means a human author.
- **IP-1:** Slop never promises that an app is protected by copyright, or that its licence is enforceable. The Publisher Agreement places this on the maker.

---

## 8. Privacy and data

### 8.1 GDPR and UK GDPR — [Settled]
GDPR applies to Slop because it offers services to people in the EU (Art. 3(2)). The UK GDPR applies the same way for the UK.

**Lawful basis for each kind of processing** (to go in the Privacy Policy and the record of processing, Art. 30):

| Processing | Basis | Notes |
|---|---|---|
| Account, library, installs, chat delivery | Contract (Art. 6(1)(b)) | |
| Public votes and reviews | Contract (it's the feature) / legitimate interest | Users can make them private. **For UK under-18s, private by default** (PRIV-6) |
| Pulse playtime and vote weighting | Contract (for the user's own playtime shown in their library) / legitimate interest (ratings) | Also needs the ePrivacy check in §8.2 |
| Recommendations ("For you") | Legitimate interest, with a "don't personalise" switch (Art. 21) | **Off by default for UK under-18s** (PRIV-6) |
| Fraud and abuse checks, security logs | Legitimate interest | |
| UK age checks | Legal obligation (UK OSA) | The provider keeps the evidence. Slop keeps only the result |
| Payment and tax records, DAC7 | Legal obligation (Art. 6(1)(c)) | |
| Marketing emails | Consent (EU), or the "soft opt-in" for existing users (UK PECR reg. 22) | Separate from product emails |

**Rules**
- **PRIV-1:** keep a **record of processing** (Art. 30) and update it in the same pull request as any new data field. Add a field-level `purpose` comment to the schema.
- **PRIV-2:** **data protection impact assessments** (DPIAs, Art. 35) **before launch** for: UK age assurance, public votes and profiles, chat, and recommendations.
- **PRIV-3:** self-service **export and deletion**. Answer other requests **within one month** (Art. 12(3)). In the UK, offer a **data-protection complaints process** (DPA 2018 s.164A, added by the Data (Use and Access) Act 2025, in force 19 June 2026).
- **PRIV-4:** **breaches:** notify the regulator **within 72 hours** of becoming aware (Art. 33), and the users affected when the risk is high (Art. 34). Log every incident, even ones that aren't reported.
- **PRIV-5:** a **data processing agreement** (Art. 28) with every processor (cloud, email, age-check provider, error tracking). A list of sub-processors in the Privacy Policy.
- **International transfers:** IronPoint is a US company. Use the **EU-US Data Privacy Framework** (self-certify) plus the UK Extension. **Also keep standard contractual clauses ready.** The Framework was upheld by the EU General Court in September 2025, but an appeal is pending at the Court of Justice (*Latombe*, C-703/25 P). If it falls, the clauses are the fallback.
- **Representatives:** an EU representative and a UK representative (Art. 27) before launch.

### 8.2 Cookies, telemetry and Pulse — [Our reading]
**The law.** Reading or storing information on a user's device needs **consent** unless it's **strictly necessary** for a service the user asked for. This covers apps and SDK telemetry, not just browser cookies:
- **EU:** ePrivacy Directive Art. 5(3) (EDPB Guidelines 2/2023 confirm it covers apps and SDK telemetry);
- **Germany:** TDDDG §25;
- **UK:** PECR reg. 6.

The UK eased this from **5 February 2026** (Data (Use and Access) Act 2025): some **analytics, security and functionality** uses no longer need consent if users are told and **can opt out**.

**Rules**
- **PRIV-7:** the website sets only essential cookies without consent. Analytics are cookieless and aggregated, or need consent in the EU (UK: tell users and give an opt-out).
- **PRIV-8:** the client and the Pulse SDK send only what the feature needs: app ID, launch token, session start, heartbeats. No device IDs, no gameplay data, no hardware fingerprinting (already in `SECURITY.md`). Any new telemetry field needs a privacy review.
- **PRIV-9:** build a per-user **"Share my playtime"** setting with a **per-country default**. Whether Pulse counts as "strictly necessary" in the EU is a question for the lawyer (`LEGAL.md` §15 Q19). If the answer is consent, we switch the EU default to off with no rework. When it's off, the user's votes count as unweighted.

### 8.3 US privacy — [Settled; thresholds matter]
- **State privacy laws** apply above certain thresholds, so they probably don't cover Slop at launch. California's CCPA/CPRA applies with about **$26.6M** in annual revenue (inflation-adjusted), personal data of 100,000+ California residents, or 50% of revenue from selling data. Other states have similar laws.
  - **PRIV-10:** build to the CCPA standard anyway:
    - honour **Global Privacy Control** signals;
    - never sell or "share" personal data for cross-context advertising;
    - the same export and delete tools as for GDPR.
- **COPPA** (16 CFR Part 312, amended rule with compliance by **22 April 2026**) applies when a service has **actual knowledge** that a user is under 13.
  - **PRIV-11:** an account found to be under 16 is deleted (`LEGAL.md` §6). Delete it quickly and log it.
- **Biometric privacy:** Illinois BIPA (740 ILCS 14) requires written consent before collecting face geometry. Texas and Washington have similar laws.
  - **PRIV-12:** **facial age estimation is used only in the UK flow** (§9). Never turn it on for US users without the lawyer's sign-off.

### 8.4 Children's data in the UK — [Settled]
The ICO's **Age Appropriate Design Code** (Children's Code, DPA 2018 s.123) applies to services likely to be used by **under-18s** in the UK. Slop allows UK users aged 16–17.

- **PRIV-6:** for UK accounts in the under-18 view:
  - **profile, votes and reviews private by default** (standard 7). This is an exception to the public-by-default decision;
  - **recommendation profiling off by default** (standard 12);
  - **no precise location** (standard 10);
  - **no nudges** to share more or turn off privacy settings (standard 13);
  - the privacy policy is written so a 16-year-old can understand it (standard 4).

---

## 9. Age and children

### 9.1 What applies where

| Where | Law | What Slop does |
|---|---|---|
| Everywhere | Slop's rule: 16+. GDPR Art. 8 (16 is the highest digital-consent age in the EU). COPPA (under 13) | Date-of-birth confirmation at sign-up. Delete accounts found to be under 16 |
| Everywhere | Contracts and Stripe: payments need adults | Donating and Humans are **18+**, confirmed on first use |
| **UK** | Online Safety Act children's duties; Children's Code | **Under-18 view by default**. **Highly effective age assurance** unlocks 18+ content. Safe contact defaults (§9.2) |
| **Germany** | JMStV §5: content that may harm a minor's development (e.g. games rated 18) must be kept from minors by **technical means**. JuSchG §14a (age labels on game platforms) applies only once a platform has **1 million or more users in Germany** | ⚖️ Possibly apply the under-18 view to Germany too. **AGE-2** makes this a setting |
| **EU generally** | DSA Art. 28 (small enterprises exempt) and the Commission's 2025 guidelines on protecting minors | Follow the UK defaults across the EU where cheap |
| **US states** | App-store age laws (§9.4). Laws on minors and social media (some blocked in court) | Per-state configuration (§9.4) |

**Rules**
- **AGE-1:** age logic lives in a **per-country (and per-US-state) policy table**, with these settings:
  - minimum age;
  - whether the under-18 view is the default;
  - which age-check methods unlock 18+ content;
  - contact defaults;
  - whether payments are allowed.

  No age rules in feature code.
- **AGE-2:** the UK under-18 view is **one policy setting** that can be turned on for any other country (Germany first) without code changes.
- **AGE-3:** Slop stores only the **age-check result** ("UK age check passed", date, method, provider reference), never ID documents or face images.

### 9.2 UK under-18 safe defaults (Ofcom's Protection of Children Codes)
- **AGE-4:**
  - 18+ apps, screenshots, reviews and store-page content are hidden;
  - only existing connections (friends, team members, Humans counterparts) can send messages, and this **can't be changed** while in the under-18 view;
  - the user isn't suggested to strangers;
  - block and report are one tap away;
  - reports involving a child go to the **front of the moderation queue**.
- **AGE-5:** **logged-out UK visitors** see the under-18 view.
- **AGE-6:** age-assurance methods are "highly effective" types only: facial age estimation, photo-ID matching, open banking, a credit-card check, mobile network checks or a digital ID. **Self-declaration and ticking a box don't count.**

### 9.3 Content ratings
- **AGE-7:** makers fill in an IARC-style questionnaire. The resulting rating drives what the under-18 view hides. Keep the rating **per release**, because content can change between versions.

### 9.4 US app-store age laws — [In flux]
These laws treat any service that **distributes apps to mobile devices** as an "app store". Slop's phone app installs Android apps ("Send to phone"), so they **probably cover Slop's Android distribution**. Among other things they require:
- age verification;
- **parental consent for users under 18** (Slop allows 16–17-year-olds);
- age signals passed to developers.

| State | Status (checked 2026-10-17) |
|---|---|
| **Texas** (SB 2420) | Blocked by a district court in December 2025. **In effect since June 2026** after the Fifth Circuit stayed that injunction. The Supreme Court declined to lift the stay. The appeal is still pending |
| **Utah** (SB 142) | Main duties from May 2026. Reports of litigation conflict, so check |
| **Louisiana** (HB 570) | From 1 July 2026 |
| **California** (AB 1043, Digital Age Assurance Act) | From **1 January 2027**. App stores and operating systems pass on an age bracket. Developers must act on it |
| **Federal** (App Store Accountability Act) | A bill, not law |

**Rules**
- **AGE-8:** until the lawyer clears it, **turn off phone app installs** ("Send to phone" for Android) for users in Texas, Utah and Louisiana, and from 1 January 2027 in California. Use the per-state policy table (AGE-1). Desktop isn't affected.
- **AGE-9:** Slop's own phone app reads the platform age signals where they exist (Apple's Declared Age Range, Google Play's age signals), so it's ready for California in 2027.

---

## 10. Chat and law-enforcement requests

**The law.**
- **End-to-end encryption is lawful** in the US, the EU and the UK. No law currently requires breaking it.
- **Private messages are confidential in the EU.** Chat between users is a "number-independent interpersonal communications service" (EECC, Directive (EU) 2018/1972, Art. 2(7)). That means the ePrivacy Directive protects message content (Art. 5(1)) and limits use of metadata (Art. 6).
- **EU scanning law:** the interim rules allowing *voluntary* scanning for child abuse material were reinstated until **3 April 2028** (Regulation (EU) 2026/1881). They **don't apply to E2E-encrypted communications**. The permanent CSA Regulation ("chat control") is still being negotiated.
- **UK:** the government can serve notices that could require changes to encryption:
  - technical capability notices under the Investigatory Powers Act 2016 (Apple withdrew its Advanced Data Protection feature from the UK in February 2025 over one);
  - OSA s.121 notices, which Ofcom hasn't used.
- **US:** the Stored Communications Act (18 U.S.C. §§2701–2713) controls disclosure:
  - **message content needs a warrant**;
  - providers must **preserve records for 90 days** on request, extendable by another 90 (§2703(f));
  - **emergency disclosure** is voluntary (§2702).

**Rules**
- **CHAT-1:** no server-side keys, no plaintext on servers, no server-side content scanning. Reports send only what the reporter's app chooses to send, with the reporter's consent (message franking proves it's genuine).
- **CHAT-2:** **keep as little metadata as possible.** Delivery queues are deleted once messages are delivered. Keep connection logs (IP, time) for a short, fixed period (proposed: 30 days, ⚖️) and only for security.
- **CHAT-3:** a written **law-enforcement request policy**:
  - who reviews requests;
  - what each kind of legal order allows: US subpoena, court order or warrant; EU production order; UK requests via the US-UK Data Access Agreement;
  - emergency requests;
  - notifying users unless that's legally barred;
  - a log of every request.

  Slop can only give what it has: account details, metadata it keeps, and reported messages.
- **CHAT-4:** publish a yearly **transparency report**: number of requests by country and type, and how many were answered.
- **CHAT-5:** keep the cryptography in the **client**, so that if a government ever makes demands, the honest answer remains "we can't read messages". Don't add features that would change that answer (cloud search of messages, server-side previews) without the lawyer (§2).

---

## 11. Accessibility — [Settled]
**The law.**
- **EU:** the **European Accessibility Act** (Directive (EU) 2019/882, applying since 28 June 2025) covers e-commerce services. **Microenterprises** providing services (fewer than 10 staff and €2M or less in turnover or balance sheet) are exempt (Art. 4(5)). The technical standard is EN 301 549, which follows WCAG 2.1 AA.
- **UK:** service providers must make reasonable adjustments for disabled people (Equality Act 2010 ss.20, 29).
- **US:** the ADA (Title III) is applied to websites inconsistently by the courts.

**Rules**
- **A11Y-1:** build the website, client and phone app to **WCAG 2.2 AA**: keyboard navigation, screen-reader labels, contrast, captions for Slop's own videos, and no colour-only meanings (the 👍/👎 ratings also show text).
- **A11Y-2:** publish an **accessibility statement** with a contact for problems.

---

## 12. Open-source licences, trademarks and Slop's own code

- **IP-2:** every app shows its licence (SPDX ID) on the store page. The licence scan flags missing or contradictory licences, and code copied from incompatible licences.
- **IP-3:** **copyleft source:** apps under the GPL and similar licences must have the matching source available when binaries are distributed (GPLv3 §6, GPLv2 §3). Public repos meet this. Private Slop owners are responsible for their own sharees.
- **IP-4:** **Slop's own dependencies:** keep a licence inventory (the SBOM in SAFE-2), and meet each licence's notice requirements in the client's "About / Licences" screen (OpenMLS, Tauri and others).
- **IP-5:** **trademark:** clear "Slop" and the logo with searches at the USPTO, EUIPO and UKIPO before spending on branding, then register them. Don't copy Steam's name, logo, colours or overall look.
- **IP-6:** don't use makers' code, assets or users' content to **train or fine-tune AI models** (§2).

---

## 13. Reviews, ratings and ranking — [Settled]
**The law.**
- **US:** the FTC's rule on consumer reviews (16 CFR Part 465, since 21 October 2024) bans:
  - fake reviews;
  - buying reviews, or giving rewards that depend on whether a review is positive;
  - suppressing negative reviews (§465.7).
- **EU:** under the UCPD as amended by the Omnibus Directive, platforms must say **whether and how they check reviews are genuine** (Art. 7(6)) and give the **main ranking factors** (Art. 7(4a)). Paid ranking must be labelled (Annex I pt 11a). Fake reviews are banned (Annex I pts 23b–23c).
- **UK:** the DMCCA 2024 bans fake reviews, and requires platforms to take reasonable steps to **prevent and remove** them (Sch. 20, in force April 2025).

**Rules**
- **REV-1:** publish a "How ratings work" page explaining:
  - who can vote (only people who launched the app through Slop);
  - the weighting formula;
  - how fake votes are detected.
- **REV-2:** **never remove or hide a vote or review because it's negative.** Removal follows the Content Policy only, and makers can't hide reviews.
- **REV-3:** makers and creatives can't offer anything in exchange for votes or reviews. The Publisher Agreement bans it, and Pulse/vote abuse checks look for it.
- **REV-4:** there's no paid ranking. If it's ever added, it needs labelling and the lawyer first (§2).
- **REV-5:** keep **records of detection and removal** of fake votes (§15). They show the "reasonable steps" the UK requires.

---

## 14. Terms, emails and marketing — [Settled]
- **TERMS-1:** EU and UK consumers can't be bound to a foreign court or forced into arbitration in a way that takes away their home protections (EU Unfair Terms Directive 93/13/EEC; UK Consumer Rights Act 2015 Part 2). The Terms say: "If you're a consumer in the EU or UK, you keep the rights your local law gives you and can bring claims in your local courts."
- **TERMS-2:** never exclude liability for death or injury caused by negligence, or for fraud, intent or gross negligence. Those exclusions are void anyway, and they make the rest of the Terms look unfair.
- **TERMS-3:** product emails (receipts, security alerts, legal notices) are kept separate from marketing emails. Marketing needs:
  - consent in the EU, or the soft opt-in in the UK (PECR reg. 22);
  - a working unsubscribe link (US CAN-SPAM Act, 15 U.S.C. §7704).
- **TERMS-4:** marketing claims follow the word list and CON-17. No "safest", "verified", or claims about player numbers that Pulse can't prove.

---

## 15. Records and how long to keep them

These records are Slop's evidence if a position is ever challenged. **Store them append-only** with access logs.

| Record | Keep for | Why |
|---|---|---|
| Slop fee, VAT and invoice records | **10 years** | EU non-Union OSS records (VAT Directive Art. 369). UK VAT needs 6 years; the US generally 7 |
| DAC7 / UK platform reporting data and reports | 10 years | DAC7 sets a 5–10-year range by country; use the longest |
| Payment records (mirror of Stripe) | 10 years | Matches tax records |
| Accepted Terms / Publisher Agreement / Humans Terms versions, per user | Life of account + **6 years** | Contract claims (6 years in England & Wales; similar or shorter in most US states) |
| Copies of every payment, donation and consent screen, per release | 10 years | Shows exactly what users saw (`LEGAL.md` §13) |
| Notices, decisions, statements of reasons, appeals | 6 years | DSA, OSA s.23, DMCA disputes |
| DMCA notices, counter-notices, repeat-infringer strikes | 6 years | §512 |
| Child-abuse reports and preserved material | **1 year** preservation (US), then as law enforcement directs | 18 U.S.C. §2258A(h). Access strictly limited |
| TAKE IT DOWN requests and removal times | 6 years | Shows the 48-hour timing was met |
| OSA risk assessments (all versions) | Indefinitely | OSA s.23 |
| Scan records per release | Life of app + 6 years | Shows reasonable care (CON-19) |
| Kill-switch and revocation actions | Life of app + 6 years | SAFE-9 |
| CRA vulnerability reports and SBOMs | **10 years** after the release, or the support period if longer | CRA Art. 13 (technical documentation) |
| Law-enforcement requests and responses | 6 years | CHAT-3 |
| UK age-check result (not the evidence) | Life of account | AGE-3 |
| Security logs with IP addresses | 30–90 days (⚖️) | Keep as little as possible (GDPR Art. 5(1)(e)) |
| Chat delivery metadata | Shortest workable (proposed 30 days, ⚖️) | ePrivacy Art. 6; CHAT-2 |
| Deleted accounts' votes | Anonymised, kept | `METRICS.md` §6 |

---

## 16. Before public launch: legal checklist

**Company and registrations**
- [ ] Wyoming LLC in good standing, with a registered agent and EIN. Foreign-LLC registration where the founders work (⚖️).
- [ ] EU GDPR representative, UK GDPR representative, DSA legal representative, TCO Regulation legal representative and contact point, and the e-Evidence addressee (one firm can often cover all of these).
- [ ] DMCA designated agent registered (CON-2).
- [ ] EU non-Union OSS registration and UK VAT registration (from the first fee).
- [ ] DAC7 registration in one EU country, and HMRC platform-reporting registration (before Humans goes live).
- [ ] NCMEC CyberTipline registration and NCA registration (CON-6, CON-14).
- [ ] Insurance: technology E&O, and cyber insurance.

**Documents** (list in `LEGAL.md` §11)
- [ ] Terms of Service, Donation Terms, Publisher Agreement, Humans Terms with the default Service Agreement, Official Team Terms.
- [ ] Privacy Policy, cookie notice, sub-processor list, Content & Conduct Policy, Copyright & Trademark Policy.
- [ ] "What scanning means", "How ratings work", "How ranking works", accessibility statement, vulnerability disclosure policy + `security.txt`, law-enforcement policy.

**Assessments**
- [ ] UK OSA illegal content risk assessment, children's access assessment and children's risk assessment (CON-13).
- [ ] DPIAs: age assurance, public profiles and votes, chat, recommendations (PRIV-2).
- [ ] Record of processing (PRIV-1).
- [ ] Trademark clearance (IP-5).

**Product**
- [ ] Direct charges only, with daily reconciliation (PAY-1, PAY-2).
- [ ] All-in totals shown first; `donor_covers_fees` setting (PAY-10, PAY-12).
- [ ] Server-donation consent, reminders and one-step cancellation (PAY-14 to PAY-17).
- [ ] Trader declaration, withdrawal information and withdrawal button for Humans (HUM-1 to HUM-3).
- [ ] Report forms, statements of reasons, appeals, 48-hour NCII process, 1-hour terrorist-content process (CON-5, CON-8, CON-9, CON-11).
- [ ] Banned-word lint (CON-17) and the "no donation field in app APIs" test (PAY-5).
- [ ] Per-country/state policy table with the UK under-18 view, Germany ready, and US phone installs gated (AGE-1, AGE-2, AGE-8).
- [ ] UK under-18 privacy defaults (PRIV-6).
- [ ] CRA reporting process and SBOMs (SAFE-1, SAFE-2).
- [ ] Records and retention jobs (§15).

---

## 17. Dates to watch

**Coming up**

| Date | What | Section |
|---|---|---|
| **2 Dec 2026** | AI Act: new bans on nudify apps and on AI that generates child abuse material | §5.4, §7.1 |
| **2 Dec 2026** | Platform Work Directive must be in EU countries' national law | §4.3 |
| **9 Dec 2026** | New Product Liability Directive applies (software is a product) | §6.1 |
| **1 Jan 2027** | California Digital Age Assurance Act (AB 1043) | §9.4 |
| Jan 2027 (reported; earlier: spring 2027) | UK DMCCA subscription-contracts regime | §3.4 |
| **2 Dec 2027** | AI Act high-risk rules (Annex III) | §7.1 |
| **11 Dec 2027** | Cyber Resilience Act fully applies (CE marking, security requirements) | §6.1 |
| **3 Apr 2028** | EU interim CSAM-scanning rules expire | §10 |
| **2 Aug 2028** | AI Act high-risk rules for regulated products (Annex I) | §7.1 |

**Pending, no date yet:**
- Court of Justice ruling on the Data Privacy Framework (C-703/25 P);
- the EU **Digital Fairness Act** proposal (expected 2026: dark patterns, addictive design, subscriptions);
- the EU CSA Regulation;
- PSD3 / Payment Services Regulation;
- the Fifth Circuit's main ruling on Texas SB 2420;
- federal US bills (App Store Accountability Act, KOSA);
- UK reform of computer-generated works.

**Every year:**
- **31 January:** DAC7 and UK platform reports.
- **Quarterly:** non-Union OSS VAT returns; review the sanctions and Stripe restricted lists.
- **Yearly:** review the OSA risk assessments; transparency report.
- **Every 3 years:** renew the DMCA agent.
- **Every 6 months:** publish EU monthly-active-user numbers. Small enterprises may be exempt, but publishing them is cheap and shows good faith.

---

## 18. Where `LEGAL.md` is out of date (for the lawyer review)

`LEGAL.md` wasn't changed, so the lawyer reviews the draft as written. These are the corrections this review found:
1. **§5.8, AI Act table:** the Digital Omnibus was **adopted** (Regulation (EU) 2026/1744, in force 27 July 2026).
   - Annex III high-risk rules now start **2 Dec 2027**; Annex I rules **2 Aug 2028**.
   - New Art. 5 bans (nudify apps, AI that generates child abuse material) apply from **2 Dec 2026**.
   - Art. 4 has been softened.
   - Art. 50 wasn't delayed.
2. **§5.3 / §5.9:** the UK **CSEA reporting duty to the NCA** has been in force since **7 April 2026** (SI 2026/268).
3. **§4.1:** add the EU **withdrawal button** (CRD Art. 11a, from 19 June 2026) and the trader declaration (CRD Art. 6a) for Humans.
4. **§4.6:** add the **EU e-Evidence** addressee (applying since 18 Aug 2026), and note that the reinstated interim CSAM rules exclude E2E chat (Regulation (EU) 2026/1881).
5. **§6 and §7:** the **Children's Code** means UK under-18 profiles, votes and reviews should be **private by default**, with recommendations off by default. That's a change to the "public by default" decision for that group.
6. **§5.7:** Germany's JuSchG §14a labelling duty starts at 1 million users in Germany. JMStV §5 access controls for 18+ content may apply from the start.
7. **§15 Q34:** Slop's Android "Send to phone" probably makes it a mobile app store under the state laws. Texas's law is **in effect** (since the Fifth Circuit stay, June 2026). California AB 1043 starts 1 Jan 2027.
8. **New topics** to add:
   - the Slop client's own CRA duties (reporting since 11 Sep 2026);
   - the Digital Content Directive for Slop's own free service;
   - the ePrivacy question for Pulse (PRIV-9);
   - the UK DUAA cookie exemptions and data-protection complaints duty.
9. **§3.1:** the main money-transmission argument is that Slop **never accepts** the funds (31 CFR 1010.100(ff)(5)). The payment-processor exemption speaks of buying goods and services, so it fits donations less well.

---

## 19. Sources (official texts)

**EU**
- Digital Services Act: [Regulation (EU) 2022/2065](https://eur-lex.europa.eu/eli/reg/2022/2065/oj)
- AI Act: [Regulation (EU) 2024/1689](https://eur-lex.europa.eu/eli/reg/2024/1689/oj); amended by the Digital Omnibus on AI, [Regulation (EU) 2026/1744](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=OJ:L_202601744)
- Cyber Resilience Act: [Regulation (EU) 2024/2847](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)
- Product Liability Directive: [Directive (EU) 2024/2853](https://eur-lex.europa.eu/eli/dir/2024/2853/oj)
- GDPR: [Regulation (EU) 2016/679](https://eur-lex.europa.eu/eli/reg/2016/679/oj) · ePrivacy: [Directive 2002/58/EC](https://eur-lex.europa.eu/eli/dir/2002/58/oj)
- Consumer Rights Directive: [Directive 2011/83/EU](https://eur-lex.europa.eu/eli/dir/2011/83/oj) · Omnibus: [Directive (EU) 2019/2161](https://eur-lex.europa.eu/eli/dir/2019/2161/oj) · Withdrawal button: [Directive (EU) 2023/2673](https://eur-lex.europa.eu/eli/dir/2023/2673/oj)
- Unfair Commercial Practices: [Directive 2005/29/EC](https://eur-lex.europa.eu/eli/dir/2005/29/oj) · Digital Content: [Directive (EU) 2019/770](https://eur-lex.europa.eu/eli/dir/2019/770/oj)
- P2B Regulation: [Regulation (EU) 2019/1150](https://eur-lex.europa.eu/eli/reg/2019/1150/oj) · Terrorist content: [Regulation (EU) 2021/784](https://eur-lex.europa.eu/eli/reg/2021/784/oj)
- E-evidence: [Regulation (EU) 2023/1543](https://eur-lex.europa.eu/eli/reg/2023/1543/oj) and [Directive (EU) 2023/1544](https://eur-lex.europa.eu/eli/dir/2023/1544/oj)
- Platform Work: [Directive (EU) 2024/2831](https://eur-lex.europa.eu/eli/dir/2024/2831/oj) · DAC7: [Directive (EU) 2021/514](https://eur-lex.europa.eu/eli/dir/2021/514/oj)
- VAT Directive: [2006/112/EC](https://eur-lex.europa.eu/eli/dir/2006/112/oj) · VAT Implementing Regulation: [282/2011](https://eur-lex.europa.eu/eli/reg_impl/2011/282/oj)
- PSD2: [Directive (EU) 2015/2366](https://eur-lex.europa.eu/eli/dir/2015/2366/oj) · Accessibility Act: [Directive (EU) 2019/882](https://eur-lex.europa.eu/eli/dir/2019/882/oj)

**UK**
- [Online Safety Act 2023](https://www.legislation.gov.uk/ukpga/2023/50) · [Ofcom online safety guidance](https://www.ofcom.org.uk/online-safety)
- [Digital Markets, Competition and Consumers Act 2024](https://www.legislation.gov.uk/ukpga/2024/13) · [Consumer Contracts Regulations 2013](https://www.legislation.gov.uk/uksi/2013/3134)
- [Data Protection Act 2018](https://www.legislation.gov.uk/ukpga/2018/12) · [Data (Use and Access) Act 2025](https://www.legislation.gov.uk/ukpga/2025/18) · [PECR 2003](https://www.legislation.gov.uk/uksi/2003/2426) · [ICO Children's Code](https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/childrens-information/childrens-code-guidance-and-resources/)
- [Payment Surcharges Regulations 2012](https://www.legislation.gov.uk/uksi/2012/3110) · [Platform reporting rules 2023](https://www.legislation.gov.uk/uksi/2023/817)

**US**
- [47 U.S.C. §230](https://www.law.cornell.edu/uscode/text/47/230) · [17 U.S.C. §512](https://www.law.cornell.edu/uscode/text/17/512) · [DMCA agent directory](https://www.copyright.gov/dmca-directory/)
- [18 U.S.C. §2258A](https://www.law.cornell.edu/uscode/text/18/2258A) · [Stored Communications Act, 18 U.S.C. §2701 ff.](https://www.law.cornell.edu/uscode/text/18/part-I/chapter-121)
- TAKE IT DOWN Act (Pub. L. 119-12): [FTC guidance](https://www.ftc.gov/business-guidance)
- [ROSCA, 15 U.S.C. §8401 ff.](https://www.law.cornell.edu/uscode/text/15/chapter-110) · [16 CFR Part 465 (reviews)](https://www.ecfr.gov/current/title-16/chapter-I/subchapter-D/part-465) · [COPPA Rule, 16 CFR Part 312](https://www.ecfr.gov/current/title-16/chapter-I/subchapter-C/part-312)
- [31 CFR 1010.100 (money transmitter definition)](https://www.ecfr.gov/current/title-31/subtitle-B/chapter-X/part-1010/subpart-A/section-1010.100) · [OFAC sanctions programs](https://ofac.treasury.gov/sanctions-programs-and-country-information)
- [US Copyright Office: Copyright and AI](https://www.copyright.gov/ai/)

**Status checks used for this version (secondary sources, 2026):** Digital Omnibus adoption and dates (Cooley, Cuatrecasas, Gibson Dunn); Texas SB 2420 litigation (CCIA, Morrison Foerster); UK subscription regime timing (Stephenson Harwood, Brodies, Taylor Wessing); e-Evidence (Reed Smith); OSA CSEA reporting (Ofcom roadmap, SI 2026/268); interim CSAM regulation (eucrim); TAKE IT DOWN enforcement (Covington, WilmerHale); DPF appeal (WilmerHale); DUAA commencement (Bratby Law). **Re-check these before relying on them.** Several were still changing at the time of writing.
