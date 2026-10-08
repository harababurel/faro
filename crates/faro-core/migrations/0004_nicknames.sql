-- Nicknames (D65): a name the user gives someone, kept only in faro and
-- shown instead of their profile or address-book name.
ALTER TABLE recipients ADD COLUMN nickname TEXT;
