---
title: Billing - Neo Zed AI
description: Manage Neo Zed AI billing, payment methods, invoices, threshold billing, and sales tax information.
---

# Billing

This page covers billing for Neo Zed's [subscription plans](./subscription.md). For details on what's included in each plan and how token usage works, see [Plans and Usage](./plans-and-usage.md).

We use Stripe as our payments provider, and Orb for invoicing and metering. All Pro plans require payment via credit card or other supported payment method.
For invoice-based billing, a Business plan is required. Contact [sales@neozed.dev](mailto:sales@neozed.dev) for more information.

## Billing Information {#settings}

Access billing information and settings at [neozed.dev/account](https://neozed.dev/account).
This page embeds data from Orb, our invoicing and metering partner.

## Billing Cycles {#billing-cycles}

Neo Zed is billed on a monthly basis based on the date you initially subscribe. You'll receive _at least_ one invoice from Neo Zed each month you're subscribed to Neo Zed Pro, and more than one if you use more than $10 in incremental token spend within the month.

## Threshold Billing {#threshold-billing}

Neo Zed utilizes threshold billing to ensure timely collection of owed monies and prevent abuse. Every time your usage of Neo Zed's hosted models crosses a $10 spend threshold, a new invoice is generated, and the threshold resets to $0.

For example,

- You subscribe on February 1. Your first invoice is $10.
- You use $12 of incremental tokens in the month of February, with the first $10 spent on February 15. You'll receive an invoice for $10 on February 15
- On March 1, you receive an invoice for $12: $10 (March Pro subscription) and $2 in leftover token spend, since your usage didn't cross the $10 threshold.

## Payment Failures {#payment-failures}

If payment of an invoice fails, Neo Zed will block usage of our hosted models until the payment is complete. Email [billing-support@neozed.dev](mailto:billing-support@neozed.dev) for assistance.

## Invoice History {#invoice-history}

You can access your invoice history by navigating to [neozed.dev/account](https://neozed.dev/account) and clicking `Invoice history` within the embedded Orb portal.

If you require historical Stripe invoices, email [billing-support@neozed.dev](mailto:billing-support@neozed.dev)

## Updating Billing Information {#updating-billing-info}

Email [billing-support@neozed.dev](mailto:billing-support@neozed.dev) for help updating payment methods, names, addresses, and tax information.

> Self-service billing updates will be available in a future release.

Please note that changes to billing information will **only** affect future invoices — **we cannot modify historical invoices**.

## Sales Tax {#sales-tax}

Neo Zed partners with [Sphere](https://www.getsphere.com/) to calculate indirect tax rate for invoices, based on customer location and the product being sold. Tax is listed as a separate line item on invoices, based preferentially on your billing address, followed by the card issue country known to Stripe.

If you have a VAT/GST ID, you can add it at during checkout. Check the box that denotes you as a business.

Please note that changes to VAT/GST IDs and address will **only** affect future invoices — **we cannot modify historical invoices**.
Questions or issues can be directed to [billing-support@neozed.dev](mailto:billing-support@neozed.dev).
