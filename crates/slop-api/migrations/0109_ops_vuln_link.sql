-- Point the known-vulnerable dependencies alert and its existing notifications straight to the
-- vulnerabilities view in Admin instead of Admin Home.
update ops_alerts
   set link = '/admin?tab=queue&alert=dependencies-vulnerable'
 where key = 'dependencies-vulnerable'
   and link = '/admin';

update notifications
   set link = '/admin?tab=queue&alert=dependencies-vulnerable'
 where collapse_key = 'ops:dependencies-vulnerable'
   and link = '/admin';
